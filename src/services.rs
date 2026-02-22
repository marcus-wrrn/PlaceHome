use std::collections::HashMap;
use serde::Serialize;
use tokio::process::Command;
use tracing::{info, warn};

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

/// Metadata about a binary discovered on the host system.
///
/// Produced by [`detect_binary`] and stored inside [`ServiceCapabilities`].
#[derive(Debug, Clone)]
pub struct BinaryInfo {
    pub path: String,
    pub version: Option<String>,
}

/// A snapshot of which required binaries are available on the host system.
///
/// Built once at startup via [`detect_capabilities`] and then passed
/// read-only to anything that needs to know whether a dependency is
/// present before attempting to launch it.
#[derive(Debug, Clone)]
pub struct ServiceCapabilities {
    /// Map from binary name (e.g. `"mosquitto"`) to its detected info,
    /// or `None` if the binary was not found.
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

/// Checks whether `name` exists on `$PATH` via `which` and collects its
/// version string.
///
/// Returns `None` if the binary cannot be located.
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

/// Probes the host system for all required binaries and returns a
/// [`ServiceCapabilities`] snapshot.
///
/// This should be called once during application startup. Each binary is
/// located with `which` and its version string is collected via
/// `--version`. Results are logged at `INFO` (found) or `WARN` (missing).
pub async fn detect_capabilities(required_binaries: &[&str]) -> ServiceCapabilities {

    let mut binaries = HashMap::new();

    for name in required_binaries {
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
