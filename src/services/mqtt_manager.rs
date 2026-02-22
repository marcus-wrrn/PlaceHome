use tokio::sync::mpsc;

use super::mosquitto_client::{
    MqttClientConfig, MqttClientHandle, MqttClientService, MqttCommand, MqttMessage,
    MqttMessageReceiver, MqttOutboundSender,
};

const CMD_CAPACITY: usize = 64;
const MSG_CAPACITY: usize = 64;

pub struct MqttManager {
    pub handle: MqttClientHandle,
    pub inbound_rx: MqttMessageReceiver,
    pub outbound_tx: MqttOutboundSender,
    pub service: MqttClientService,
}

impl MqttManager {
    pub fn new(config: MqttClientConfig) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<MqttCommand>(CMD_CAPACITY);
        let (msg_tx, msg_rx) = mpsc::channel::<MqttMessage>(MSG_CAPACITY);
        let (out_tx, out_rx) = mpsc::channel::<MqttMessage>(MSG_CAPACITY);

        let service = MqttClientService::new(config, cmd_tx.clone(), cmd_rx, msg_tx, out_rx);
        let handle = service.handle();

        Self {
            handle,
            inbound_rx: msg_rx,
            outbound_tx: out_tx,
            service,
        }
    }
}
