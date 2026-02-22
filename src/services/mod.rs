pub mod capabilities;
pub mod mosquitto_brokerage_mngmt;
pub mod mosquitto_client;

pub use capabilities::{detect_capabilities, ServiceCapabilities, BinaryInfo};

use serde::Serialize;

/// Identifiers for all managed services.
///
/// Used as keys when referencing services throughout the process manager
/// and any associated state maps.
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceId {
    Mosquitto,
    MqttClient,
}
