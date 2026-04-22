pub mod manager;

use async_trait::async_trait;
use bytes::Bytes;
use rumqttc::{AsyncClient, Event, EventLoop, MqttOptions, Packet, QoS, Transport};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info, warn};

use crate::config::MqttClientConfig;
use crate::infra::ca::CaService;
use crate::supervisor::ManagedService;

pub async fn provision_node_identity(ca: &CaService, cfg: &MqttClientConfig) -> Result<(), String> {
    let (cert_pem, key_pem) = ca.ensure_node_identity().await?;
    if let Some(parent) = cfg.certfile.parent() {
        tokio::fs::create_dir_all(parent).await
            .map_err(|e| format!("Failed to create cert dir: {e}"))?;
    }
    tokio::fs::write(&cfg.certfile, &cert_pem).await
        .map_err(|e| format!("Failed to write node cert: {e}"))?;
    tokio::fs::write(&cfg.keyfile, &key_pem).await
        .map_err(|e| format!("Failed to write node key: {e}"))?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct MqttMessage {
    pub topic: String,
    pub payload: Bytes,
}

pub type MqttMessageSender = mpsc::Sender<MqttMessage>;
pub type MqttMessageReceiver = mpsc::Receiver<MqttMessage>;
pub type MqttOutboundSender = mpsc::Sender<MqttMessage>;
pub type MqttOutboundReceiver = mpsc::Receiver<MqttMessage>;

/// A single MQTT topic the client must subscribe to, paired with its required QoS level.
#[derive(Debug, Clone)]
pub struct TopicSubscription {
    pub topic: &'static str,
    pub qos: QoS,
}

/// Returns the canonical list of topics this node must subscribe to on startup.
pub fn required_subscriptions() -> Vec<TopicSubscription> {
    vec![
        TopicSubscription { topic: "registration", qos: QoS::AtLeastOnce },
    ]
}

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

#[derive(Clone)]
pub struct MqttClientHandle {
    tx: mpsc::Sender<MqttCommand>,
}

impl MqttClientHandle {
    /// Sends a subscribe request to the MQTT client for the given topic and QoS level.
    /// Waits for acknowledgement from the client task before returning.
    pub async fn subscribe(&self, topic: impl Into<String>, qos: QoS) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(MqttCommand::Subscribe { topic: topic.into(), qos, reply })
            .await
            .map_err(|_| "mqtt client channel closed".to_string())?;
        rx.await.map_err(|_| "mqtt client dropped reply".to_string())?
    }

    /// Subscribes to every topic in `subscriptions` in order, returning the first error encountered.
    pub async fn subscribe_all(&self, subscriptions: &[TopicSubscription]) -> Result<(), String> {
        for sub in subscriptions {
            self.subscribe(sub.topic, sub.qos).await?;
        }
        Ok(())
    }

    /// Sends an unsubscribe request to the MQTT client for the given topic.
    /// Waits for acknowledgement from the client task before returning.
    pub async fn unsubscribe(&self, topic: impl Into<String>) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(MqttCommand::Unsubscribe { topic: topic.into(), reply })
            .await
            .map_err(|_| "mqtt client channel closed".to_string())?;
        rx.await.map_err(|_| "mqtt client dropped reply".to_string())?
    }

    /// Sends a publish request to the MQTT client for the given topic, QoS level, and payload.
    /// Waits for acknowledgement from the client task before returning.
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

pub struct MqttClientService {
    config: MqttClientConfig,
    cmd_tx: mpsc::Sender<MqttCommand>,
    cmd_rx: Option<mpsc::Receiver<MqttCommand>>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    msg_tx: MqttMessageSender,
    out_rx: Option<MqttOutboundReceiver>,
    connected_tx: Option<oneshot::Sender<()>>,
}

impl MqttClientService {
    /// Creates a new `MqttClientService` with the provided configuration and pre-constructed
    /// channel endpoints. The service is not started until [`ManagedService::start`] is called.
    pub fn new(
        config: MqttClientConfig,
        cmd_tx: mpsc::Sender<MqttCommand>,
        cmd_rx: mpsc::Receiver<MqttCommand>,
        msg_tx: MqttMessageSender,
        out_rx: MqttOutboundReceiver,
        connected_tx: oneshot::Sender<()>,
    ) -> Self {
        Self {
            config,
            cmd_tx,
            cmd_rx: Some(cmd_rx),
            shutdown_tx: None,
            msg_tx,
            out_rx: Some(out_rx),
            connected_tx: Some(connected_tx),
        }
    }

    /// Returns a cloneable [`MqttClientHandle`] that can be used to send commands to this service.
    pub fn handle(&self) -> MqttClientHandle {
        MqttClientHandle { tx: self.cmd_tx.clone() }
    }

    /// Consumes the command and outbound receivers from their `Option` fields.
    /// Returns an error if either receiver has already been taken (i.e. the service was
    /// previously started and the receivers were already moved into the spawned tasks).
    fn take_channels(
        &mut self,
    ) -> Result<(mpsc::Receiver<MqttCommand>, MqttOutboundReceiver, oneshot::Sender<()>), String> {
        let cmd_rx = self
            .cmd_rx
            .take()
            .ok_or("MqttClientService command receiver already consumed")?;
        let out_rx = self
            .out_rx
            .take()
            .ok_or("MqttClientService outbound receiver already consumed")?;
        let connected_tx = self
            .connected_tx
            .take()
            .ok_or("MqttClientService connected sender already consumed")?;
        Ok((cmd_rx, out_rx, connected_tx))
    }

    /// Constructs [`MqttOptions`] from the service configuration.
    /// Sets the keep-alive interval and credentials, and conditionally reads the CA certificate
    /// from disk and configures a TLS transport when `tls_enabled` is set in the config.
    async fn build_mqtt_opts(&self) -> Result<MqttOptions, String> {
        let mut opts =
            MqttOptions::new(&self.config.client_id, &self.config.host, self.config.port);
        opts.set_keep_alive(std::time::Duration::from_secs(30));

        if self.config.tls_enabled {
            let ca = tokio::fs::read(&self.config.cafile).await.map_err(|e| {
                format!("Failed to read CA cert '{}': {}", self.config.cafile.display(), e)
            })?;
            let cert = tokio::fs::read(&self.config.certfile).await.map_err(|e| {
                format!("Failed to read client cert '{}': {}", self.config.certfile.display(), e)
            })?;
            let key = tokio::fs::read(&self.config.keyfile).await.map_err(|e| {
                format!("Failed to read client key '{}': {}", self.config.keyfile.display(), e)
            })?;
            opts.set_transport(Transport::tls(ca, Some((cert, key)), None));
        } else {
            opts.set_credentials(&self.config.username, &self.config.password);
        }

        Ok(opts)
    }

    /// Spawns the eventloop task responsible for driving the rumqttc event loop.
    /// The task multiplexes three concurrent concerns via `tokio::select!`:
    /// - Shutdown signal: disconnects cleanly and exits.
    /// - Outbound messages: publishes messages received from `out_rx` to the broker.
    /// - Inbound events: forwards incoming `Publish` packets to `msg_tx` for downstream consumers.
    fn spawn_eventloop_task(
        client: AsyncClient,
        mut eventloop: EventLoop,
        mut out_rx: MqttOutboundReceiver,
        msg_tx: MqttMessageSender,
        mut shutdown_rx: oneshot::Receiver<()>,
        mut connected_tx: Option<oneshot::Sender<()>>,
    ) {
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        info!("MQTT client eventloop shutting down");
                        let _ = client.disconnect().await;
                        break;
                    }
                    Some(msg) = out_rx.recv() => {
                        let payload_str = std::str::from_utf8(&msg.payload).unwrap_or("<binary>");
                        debug!(topic = %msg.topic, payload = %payload_str, "MQTT outbound publish");
                        if let Err(e) = client
                            .publish(&msg.topic, QoS::AtLeastOnce, false, msg.payload)
                            .await
                        {
                            error!(topic = %msg.topic, error = %e, "MQTT outbound publish failed");
                        }
                    }
                    poll = eventloop.poll() => {
                        match poll {
                            Ok(Event::Incoming(Packet::Publish(publish))) => {
                                let payload_str = std::str::from_utf8(&publish.payload).unwrap_or("<binary>");
                                info!(topic = %publish.topic, "MQTT inbound message received");
                                debug!(topic = %publish.topic, payload = %payload_str, "MQTT inbound message payload");
                                let msg = MqttMessage {
                                    topic: publish.topic.clone(),
                                    payload: publish.payload.clone(),
                                };
                                if let Err(e) = msg_tx.send(msg).await {
                                    error!(
                                        topic = %publish.topic,
                                        "MQTT inbound queue full or closed: {}",
                                        e
                                    );
                                }
                            }
                            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                                info!("MQTT client connected to broker");
                                if let Some(tx) = connected_tx.take() {
                                    let _ = tx.send(());
                                }
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
    }

    /// Spawns the command task that processes [`MqttCommand`] messages from the handle channel.
    /// Handles subscribe, unsubscribe, and publish requests, sending results back via the
    /// oneshot reply channels. Exits when the channel is closed or a `Shutdown` command is received.
    fn spawn_command_task(client: AsyncClient, mut cmd_rx: mpsc::Receiver<MqttCommand>) {
        tokio::spawn(async move {
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
    }
}

#[async_trait]
impl ManagedService for MqttClientService {
    /// Starts the MQTT client service by building connection options, creating the rumqttc client,
    /// and spawning the eventloop and command handler tasks. Returns an error if the service is
    /// already running or if channel receivers have been previously consumed.
    async fn start(&mut self) -> Result<u32, String> {
        if self.shutdown_tx.is_some() {
            return Err("MqttClientService is already running".to_string());
        }

        let (cmd_rx, out_rx, connected_tx) = self.take_channels()?;
        let msg_tx = self.msg_tx.clone();

        let opts = self.build_mqtt_opts().await?;
        let (client, eventloop) = AsyncClient::new(opts, 10);

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        self.shutdown_tx = Some(shutdown_tx);

        Self::spawn_eventloop_task(client.clone(), eventloop, out_rx, msg_tx, shutdown_rx, Some(connected_tx));
        Self::spawn_command_task(client, cmd_rx);

        Ok(0)
    }

    /// Signals the eventloop task to disconnect from the broker and shut down.
    /// Returns an error if the service is not currently running.
    async fn stop(&mut self) -> Result<(), String> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
            Ok(())
        } else {
            Err("MqttClientService is not running".to_string())
        }
    }

    /// Returns `true` if the service is currently running (i.e. the shutdown sender is still held).
    async fn is_running(&mut self) -> bool {
        self.shutdown_tx.is_some()
    }
}
