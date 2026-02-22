use tokio::sync::mpsc;

use crate::config::MqttClientConfig;
use crate::supervisor::Supervisor;
use crate::services::ServiceId;
use super::mosquitto_client::{
    MqttClientHandle, MqttClientService, MqttCommand, MqttMessage,
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

/// Handles returned to the caller after registering the MQTT client service.
pub struct MqttManagerHandles {
    pub handle: MqttClientHandle,
    pub inbound_rx: MqttMessageReceiver,
    pub outbound_tx: MqttOutboundSender,
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

/// Build and register the MQTT client service onto the supervisor.
///
/// `available` should match the availability of the broker this client
/// connects to (e.g. `mosquitto_available`). Returns the channel handles
/// the caller needs to interact with the MQTT client.
pub fn register_onto(
    supervisor: &mut Supervisor,
    config: MqttClientConfig,
    available: bool,
) -> MqttManagerHandles {
    let manager = MqttManager::new(config);

    let handles = MqttManagerHandles {
        handle: manager.handle.clone(),
        inbound_rx: manager.inbound_rx,
        outbound_tx: manager.outbound_tx,
    };

    supervisor.register(ServiceId::MqttClient, Box::new(manager.service), available);
    handles
}
