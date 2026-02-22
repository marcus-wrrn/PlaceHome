use async_trait::async_trait;
use bytes::Bytes;
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use tokio::sync::{mpsc, oneshot};
use tracing::{error, info, warn};

use crate::config::MqttClientConfig;
use crate::supervisor::ManagedService;

#[derive(Debug, Clone)]
pub struct MqttMessage {
    pub topic: String,
    pub payload: Bytes,
}

pub type MqttMessageSender = mpsc::Sender<MqttMessage>;
pub type MqttMessageReceiver = mpsc::Receiver<MqttMessage>;
pub type MqttOutboundSender = mpsc::Sender<MqttMessage>;
pub type MqttOutboundReceiver = mpsc::Receiver<MqttMessage>;

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

pub struct MqttClientService {
    config: MqttClientConfig,
    cmd_tx: mpsc::Sender<MqttCommand>,
    cmd_rx: Option<mpsc::Receiver<MqttCommand>>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    msg_tx: MqttMessageSender,
    out_rx: Option<MqttOutboundReceiver>,
}

impl MqttClientService {
    pub fn new(
        config: MqttClientConfig,
        cmd_tx: mpsc::Sender<MqttCommand>,
        cmd_rx: mpsc::Receiver<MqttCommand>,
        msg_tx: MqttMessageSender,
        out_rx: MqttOutboundReceiver,
    ) -> Self {
        Self {
            config,
            cmd_tx,
            cmd_rx: Some(cmd_rx),
            shutdown_tx: None,
            msg_tx,
            out_rx: Some(out_rx),
        }
    }

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

        let msg_tx = self.msg_tx.clone();
        let mut out_rx = self
            .out_rx
            .take()
            .ok_or("MqttClientService outbound receiver already consumed")?;

        let mut opts = MqttOptions::new(&self.config.client_id, &self.config.host, self.config.port);
        opts.set_keep_alive(std::time::Duration::from_secs(30));

        let (client, mut eventloop) = AsyncClient::new(opts, 10);

        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
        self.shutdown_tx = Some(shutdown_tx);

        let client_clone = client.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        info!("MQTT client eventloop shutting down");
                        let _ = client_clone.disconnect().await;
                        break;
                    }
                    Some(msg) = out_rx.recv() => {
                        if let Err(e) = client_clone
                            .publish(&msg.topic, QoS::AtLeastOnce, false, msg.payload)
                            .await
                        {
                            error!(topic = %msg.topic, error = %e, "MQTT outbound publish failed");
                        }
                    }
                    poll = eventloop.poll() => {
                        match poll {
                            Ok(Event::Incoming(Packet::Publish(publish))) => {
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
