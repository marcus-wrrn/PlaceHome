use serde::{Deserialize, Serialize};
use crate::config::MqttBrokerageConfig;

/// mDNS settings advertised by the device.
#[derive(Debug, Deserialize)]
pub struct MdnsConfig {
    pub hostname: String,
    pub port: u16,
}

/// Registration payload sent by a device on the `POST /` route.
#[derive(Debug, Deserialize)]
pub struct DeviceInfo {
    pub address: String,
    pub mdns: MdnsConfig,
    /// PEM-encoded Certificate Signing Request for the device.
    pub csr_pem: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MqttTopicConfig {
    pub topic: String,
    pub qos: u8,
}

/// Broker connection details returned to a device after a successful handshake.
#[derive(Debug, Clone, Serialize)]
pub struct MqttBrokerageInfo {
    pub address: String,
    pub port: u16,
    pub topics: Vec<MqttTopicConfig>,
}

pub fn build_brokerage_info(config: &MqttBrokerageConfig) -> MqttBrokerageInfo {
    let port = if config.tls_enabled { config.mqtts_port } else { config.port };
    MqttBrokerageInfo {
        address: "localhost".to_string(),
        port,
        topics: vec![
            MqttTopicConfig { topic: "placenet/test".to_string(), qos: 1 },
        ],
    }
}
