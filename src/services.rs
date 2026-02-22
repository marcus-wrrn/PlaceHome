use std::collections::HashMap;
use serde::Serialize;
use tokio::process::Command;
use tracing::{info, warn};

/// Identifiers for all managed services
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceId {
    Mosquitto,
    MqttClient,
}

/// Information about a detected binary
#[derive(Debug, Clone)]
pub struct BinaryInfo {
    pub path: String,
    pub version: Option<String>,
}

/// Results of startup service detection
#[derive(Debug, Clone)]
pub struct ServiceCapabilities {
    pub binaries: HashMap<String, Option<BinaryInfo>>,
}

impl ServiceCapabilities {
    pub fn is_available(&self, binary: &str) -> bool {
        self.binaries
            .get(binary)
            .is_some_and(|info| info.is_some())
    }

    pub fn binary_path(&self, binary: &str) -> Option<&str> {
        self.binaries
            .get(binary)
            .and_then(|info| info.as_ref())
            .map(|info| info.path.as_str())
    }
}

/// Check if a binary exists on the system and get its path
async fn detect_binary(name: &str) -> Option<BinaryInfo> {
    let output = Command::new("which")
        .arg(name)
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Try to get version info
    let version = Command::new(&path)
        .arg("--version")
        .output()
        .await
        .ok()
        .and_then(|o| {
            // Some tools output version on stderr, some on stdout
            let text = if o.stdout.is_empty() {
                String::from_utf8_lossy(&o.stderr).to_string()
            } else {
                String::from_utf8_lossy(&o.stdout).to_string()
            };
            let first_line = text.lines().next()?.trim().to_string();
            if first_line.is_empty() { None } else { Some(first_line) }
        });

    Some(BinaryInfo { path, version })
}

/// Probe the system for all required binaries at startup
pub async fn detect_capabilities() -> ServiceCapabilities {
    let required_binaries = ["mosquitto", "mosquitto_passwd"];

    let mut binaries = HashMap::new();

    for name in &required_binaries {
        let info = detect_binary(name).await;
        match &info {
            Some(bi) => {
                info!(
                    "Found {}: {} ({})",
                    name,
                    bi.path,
                    bi.version.as_deref().unwrap_or("unknown version")
                );
            }
            None => {
                warn!("{} not found on system", name);
            }
        }
        binaries.insert(name.to_string(), info);
    }

    ServiceCapabilities { binaries }
}
