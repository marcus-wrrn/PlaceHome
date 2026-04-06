pub mod manager;
pub mod messages;

use std::time::Duration;

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

use crate::supervisor::ManagedService;
use messages::GatewayMessage;

/// Maximum number of outbound messages queued for the gateway connection.
const OUTBOUND_CAPACITY: usize = 64;

/// Initial reconnect delay. Doubles on each consecutive failure up to
/// `MAX_RECONNECT_DELAY`.
const INITIAL_RECONNECT_DELAY: Duration = Duration::from_secs(2);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(60);

/// A cloneable handle for sending messages to the cloud gateway.
#[derive(Clone)]
pub struct CloudGatewayHandle {
    tx: mpsc::Sender<GatewayMessage>,
}

impl CloudGatewayHandle {
    /// Enqueue a message to be sent to the cloud gateway.
    ///
    /// Returns an error if the service has shut down.
    pub async fn send(&self, msg: GatewayMessage) -> Result<(), String> {
        self.tx
            .send(msg)
            .await
            .map_err(|_| "cloud gateway channel closed".to_string())
    }
}

/// Manages the persistent WebSocket connection to the PlaceNet cloud gateway.
///
/// On start, the service spawns a Tokio task that:
/// 1. Connects to `gateway_url/ws` with automatic reconnection.
/// 2. Sends a `Register` frame with this server's `server_url`.
/// 3. Loops over inbound frames and logs them (future: routes to peer layer).
pub struct CloudGatewayService {
    server_url: String,
    gateway_url: String,
    outbound_tx: mpsc::Sender<GatewayMessage>,
    outbound_rx: Option<mpsc::Receiver<GatewayMessage>>,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl CloudGatewayService {
    pub fn new(server_url: String, gateway_url: String) -> Self {
        let (outbound_tx, outbound_rx) = mpsc::channel(OUTBOUND_CAPACITY);
        Self {
            server_url,
            gateway_url,
            outbound_tx,
            outbound_rx: Some(outbound_rx),
            shutdown_tx: None,
        }
    }

    pub fn handle(&self) -> CloudGatewayHandle {
        CloudGatewayHandle { tx: self.outbound_tx.clone() }
    }

    fn ws_url(&self) -> String {
        // Append /ws if the gateway URL doesn't already end with it.
        let base = self.gateway_url.trim_end_matches('/');
        if base.ends_with("/ws") {
            base.to_string()
        } else {
            format!("{}/ws", base)
        }
    }
}

#[async_trait]
impl ManagedService for CloudGatewayService {
    async fn start(&mut self) -> Result<u32, String> {
        let outbound_rx = self
            .outbound_rx
            .take()
            .ok_or("CloudGatewayService already started")?;

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        self.shutdown_tx = Some(shutdown_tx);

        let server_url = self.server_url.clone();
        let ws_url = self.ws_url();

        spawn_connection_task(server_url, ws_url, outbound_rx, shutdown_rx);

        // Not an OS child process — return 0 as a sentinel PID.
        Ok(0)
    }

    async fn stop(&mut self) -> Result<(), String> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        Ok(())
    }

    async fn is_running(&mut self) -> bool {
        self.shutdown_tx.is_some()
    }
}

/// Spawn the background task that maintains the gateway WebSocket connection.
fn spawn_connection_task(
    server_url: String,
    ws_url: String,
    mut outbound_rx: mpsc::Receiver<GatewayMessage>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    tokio::spawn(async move {
        let mut delay = INITIAL_RECONNECT_DELAY;

        'reconnect: loop {
            info!(url = %ws_url, "Connecting to cloud gateway");

            let ws_stream = tokio::select! {
                _ = &mut shutdown_rx => {
                    info!("Cloud gateway client shutting down before connect");
                    break 'reconnect;
                }
                result = connect_async(&ws_url) => match result {
                    Ok((stream, _)) => stream,
                    Err(e) => {
                        warn!(url = %ws_url, error = %e, delay_secs = delay.as_secs(),
                              "Failed to connect to cloud gateway, retrying");
                        tokio::time::sleep(delay).await;
                        delay = (delay * 2).min(MAX_RECONNECT_DELAY);
                        continue 'reconnect;
                    }
                }
            };

            // Successful connection — reset backoff.
            delay = INITIAL_RECONNECT_DELAY;
            info!(url = %ws_url, "Connected to cloud gateway");

            let (mut sink, mut stream) = ws_stream.split();

            // Send registration frame immediately.
            let register = GatewayMessage::Register { server_url: server_url.clone() };
            match serde_json::to_string(&register) {
                Ok(text) => {
                    if let Err(e) = sink.send(Message::Text(text.into())).await {
                        error!(error = %e, "Failed to send Register frame");
                        continue 'reconnect;
                    }
                }
                Err(e) => {
                    error!(error = %e, "Failed to serialise Register frame");
                    continue 'reconnect;
                }
            }

            // Drive inbound and outbound concurrently until the connection
            // closes or a shutdown signal is received.
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        info!("Cloud gateway client shutting down");
                        let _ = sink.send(Message::Close(None)).await;
                        break 'reconnect;
                    }

                    // Outbound: forward messages from the handle sender.
                    Some(msg) = outbound_rx.recv() => {
                        match serde_json::to_string(&msg) {
                            Ok(text) => {
                                if let Err(e) = sink.send(Message::Text(text.into())).await {
                                    warn!(error = %e, "Failed to send gateway message, reconnecting");
                                    break; // reconnect
                                }
                            }
                            Err(e) => error!(error = %e, "Failed to serialise gateway message"),
                        }
                    }

                    // Inbound: handle frames from the gateway.
                    msg = stream.next() => {
                        match msg {
                            Some(Ok(Message::Text(text))) => {
                                handle_inbound(&text);
                            }
                            Some(Ok(Message::Ping(data))) => {
                                let _ = sink.send(Message::Pong(data)).await;
                            }
                            Some(Ok(Message::Close(_))) | None => {
                                warn!(url = %ws_url, "Gateway connection closed, reconnecting");
                                break; // reconnect
                            }
                            Some(Err(e)) => {
                                warn!(url = %ws_url, error = %e, "Gateway WebSocket error, reconnecting");
                                break; // reconnect
                            }
                            Some(Ok(_)) => {} // binary/etc — ignore
                        }
                    }
                }
            }

            // Brief pause before reconnecting to avoid a tight loop on
            // connections that close immediately after opening.
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(MAX_RECONNECT_DELAY);
        }

        info!("Cloud gateway connection task stopped");
    });
}

/// Process a text frame received from the gateway.
fn handle_inbound(text: &str) {
    match serde_json::from_str::<GatewayMessage>(text) {
        Ok(GatewayMessage::ConnectRequest { from }) => {
            info!(from = %from, "Inbound connection request from peer via gateway");
            // TODO: route to peer connection layer once P2P/relay is wired up.
        }
        Ok(GatewayMessage::Relay { from, payload, .. }) => {
            info!(from = %from, "Inbound relay frame from peer via gateway");
            // TODO: dispatch payload to the appropriate local handler.
            let _ = payload;
        }
        Ok(GatewayMessage::Ack { ok, message }) => {
            if ok {
                info!("Gateway ack: ok");
            } else {
                warn!(message = ?message, "Gateway ack: error");
            }
        }
        Ok(other) => {
            warn!(?other, "Unexpected inbound gateway frame");
        }
        Err(e) => {
            warn!(error = %e, raw = %text, "Failed to parse inbound gateway frame");
        }
    }
}
