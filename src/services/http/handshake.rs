use serde::Deserialize;

/// mDNS settings advertised by the device.
#[derive(Debug, Deserialize)]
pub struct MdnsConfig {
    pub hostname: String,
    pub port: u16,
}

/// Registration payload sent by a device on the `POST /` route.
#[derive(Debug, Deserialize)]
pub struct DeviceInfo {
    /// `host:port` the device is reachable on (e.g. `"192.168.1.42:8883"`).
    pub address: String,
    pub mdns: MdnsConfig,
}
