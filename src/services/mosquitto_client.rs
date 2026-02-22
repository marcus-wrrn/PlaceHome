use async_trait::async_trait;
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use tokio::sync::{mpsc, oneshot};
use tracing::{error, info, warn};

use crate::supervisor::ManagedService;

pub enum MqttCommand {
    Subscribe {
        topic: String,
        qos: QoS,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Unsubscribe {
        topic: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Publish {
        topic: String,
        qos: QoS,
        payload: Vec<u8>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Shutdown,
}

/// A cloneable handle for sending commands to the MQTT client service.
#[derive(Clone)]
pub struct MqttClientHandle {
    tx: mpsc::Sender<MqttCommand>,
}

impl MqttClientHandle {
    pub async fn subscribe(&self, topic: impl Into<String>, qos: QoS) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(MqttCommand::Subscribe { topic: topic.into(), qos, reply })
            .await
            .map_err(|_| "mqtt client channel closed".to_string())?;
        rx.await.map_err(|_| "mqtt client dropped reply".to_string())?
    }

    pub async fn unsubscribe(&self, topic: impl Into<String>) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(MqttCommand::Unsubscribe { topic: topic.into(), reply })
            .await
            .map_err(|_| "mqtt client channel closed".to_string())?;
        rx.await.map_err(|_| "mqtt client dropped reply".to_string())?
    }

    pub async fn publish(
        &self,
        topic: impl Into<String>,
        qos: QoS,
        payload: impl Into<Vec<u8>>,
    ) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(MqttCommand::Publish {
                topic: topic.into(),
                qos,
                payload: payload.into(),
                reply,
            })
            .await
            .map_err(|_| "mqtt client channel closed".to_string())?;
        rx.await.map_err(|_| "mqtt client dropped reply".to_string())?
    }
}

/// Configuration for connecting the MQTT client to a local broker.
pub struct MqttClientConfig {
    pub client_id: String,
    pub host: String,
    pub port: u16,
}

/// A managed service wrapping a `rumqttc` async MQTT client.
///
/// Implements [`ManagedService`] so it can be registered with the supervisor.
/// Because the client is a Tokio task rather than an OS process, `start()`
/// returns a sentinel PID of `0`.
///
/// After starting, interact with the client through [`MqttClientHandle`]
/// obtained from [`MqttClientService::handle`].
pub struct MqttClientService {
    config: MqttClientConfig,
    cmd_tx: mpsc::Sender<MqttCommand>,
    cmd_rx: Option<mpsc::Receiver<MqttCommand>>,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl MqttClientService {
    pub fn new(config: MqttClientConfig) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        Self {
            config,
            cmd_tx,
            cmd_rx: Some(cmd_rx),
            shutdown_tx: None,
        }
    }

    /// Returns a handle for issuing subscribe/publish/unsubscribe commands.
    /// Safe to call before `start()` — commands will queue until the service
    /// is running.
    pub fn handle(&self) -> MqttClientHandle {
        MqttClientHandle { tx: self.cmd_tx.clone() }
    }
}

#[async_trait]
impl ManagedService for MqttClientService {
    async fn start(&mut self) -> Result<u32, String> {
        if self.shutdown_tx.is_some() {
            return Err("MqttClientService is already running".to_string());
        }

        let cmd_rx = self
            .cmd_rx
            .take()
            .ok_or("MqttClientService command receiver already consumed")?;

        let mut opts = MqttOptions::new(&self.config.client_id, &self.config.host, self.config.port);
        opts.set_keep_alive(std::time::Duration::from_secs(30));

        let (client, mut eventloop) = AsyncClient::new(opts, 10);

        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
        self.shutdown_tx = Some(shutdown_tx);

        // ── Eventloop task ───────────────────────────────────────────────────
        // Drives the rumqttc eventloop and dispatches incoming publishes.
        let client_clone = client.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        info!("MQTT client eventloop shutting down");
                        let _ = client_clone.disconnect().await;
                        break;
                    }
                    poll = eventloop.poll() => {
                        match poll {
                            Ok(Event::Incoming(Packet::Publish(publish))) => {
                                // TODO: replace with real dispatch when handlers are added
                                info!(
                                    topic = %publish.topic,
                                    bytes = publish.payload.len(),
                                    "MQTT message received"
                                );
                            }
                            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                                info!("MQTT client connected to broker");
                            }
                            Ok(Event::Incoming(Packet::Disconnect)) => {
                                warn!("MQTT broker disconnected");
                            }
                            Ok(_) => {}
                            Err(e) => {
                                error!("MQTT eventloop error: {}", e);
                                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                            }
                        }
                    }
                }
            }
        });

        // ── Command task ─────────────────────────────────────────────────────
        // Receives subscribe/publish/unsubscribe commands and forwards them
        // to the rumqttc client.
        tokio::spawn(async move {
            let mut cmd_rx = cmd_rx;
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    MqttCommand::Subscribe { topic, qos, reply } => {
                        let result = client
                            .subscribe(&topic, qos)
                            .await
                            .map_err(|e| format!("subscribe failed: {}", e));
                        if result.is_ok() {
                            info!(topic = %topic, "MQTT subscribed");
                        }
                        let _ = reply.send(result);
                    }
                    MqttCommand::Unsubscribe { topic, reply } => {
                        let result = client
                            .unsubscribe(&topic)
                            .await
                            .map_err(|e| format!("unsubscribe failed: {}", e));
                        if result.is_ok() {
                            info!(topic = %topic, "MQTT unsubscribed");
                        }
                        let _ = reply.send(result);
                    }
                    MqttCommand::Publish { topic, qos, payload, reply } => {
                        let result = client
                            .publish(&topic, qos, false, payload)
                            .await
                            .map_err(|e| format!("publish failed: {}", e));
                        if let Err(ref e) = result {
                            error!(topic = %topic, error = %e, "MQTT publish error");
                        }
                        let _ = reply.send(result);
                    }
                    MqttCommand::Shutdown => {
                        break;
                    }
                }
            }
        });

        // Sentinel PID — this is a Tokio task, not an OS process.
        Ok(0)
    }

    async fn stop(&mut self) -> Result<(), String> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
            Ok(())
        } else {
            Err("MqttClientService is not running".to_string())
        }
    }

    async fn is_running(&mut self) -> bool {
        self.shutdown_tx.is_some()
    }
}
