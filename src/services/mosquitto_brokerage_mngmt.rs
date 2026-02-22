use std::sync::Arc;
use async_trait::async_trait;
use tokio::process::{Child, Command};
use tokio::io::AsyncBufReadExt;
use tokio::sync::RwLock;
use tracing::{info, warn, error};

use crate::config::MqttBrokerageConfig;
use crate::supervisor::{ManagedService, Supervisor, SupervisorHandle};
use crate::services::{ServiceCapabilities, ServiceId};

/// Manages a Mosquitto MQTT broker child process
pub struct MosquittoService {
    binary_path: String,
    password_binary: Option<String>,
    config: Arc<RwLock<MqttBrokerageConfig>>,
    child: Option<Child>,
}

impl MosquittoService {
    pub fn new(
        binary_path: String,
        password_binary: Option<String>,
        config: Arc<RwLock<MqttBrokerageConfig>>,
    ) -> Self {
        Self {
            binary_path,
            password_binary,
            config,
            child: None,
        }
    }

    /// Add or update a user's password using mosquitto_passwd
    pub async fn set_password(&self, username: &str, password: &str) -> Result<(), String> {
        let passwd_bin = self.password_binary.as_deref()
            .ok_or_else(|| "mosquitto_passwd is not installed".to_string())?;

        let password_file = self.config.read().await.password_file.clone();
        if !password_file.exists() {
            // -c creates a new file, -b reads password from command line
            let output = Command::new(passwd_bin)
                .args(["-c", "-b"])
                .arg(&password_file)
                .arg(username)
                .arg(password)
                .output()
                .await
                .map_err(|e| format!("Failed to run mosquitto_passwd: {}", e))?;

            if !output.status.success() {
                return Err(format!(
                    "mosquitto_passwd failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
        } else {
            // -b reads password from command line (no -c, appends/updates)
            let output = Command::new(passwd_bin)
                .arg("-b")
                .arg(&password_file)
                .arg(username)
                .arg(password)
                .output()
                .await
                .map_err(|e| format!("Failed to run mosquitto_passwd: {}", e))?;

            if !output.status.success() {
                return Err(format!(
                    "mosquitto_passwd failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
        }

        info!("Set password for MQTT user: {}", username);
        Ok(())
    }

    /// Delete a user from the password file
    pub async fn delete_user(&self, username: &str) -> Result<(), String> {
        let passwd_bin = self.password_binary.as_deref()
            .ok_or_else(|| "mosquitto_passwd is not installed".to_string())?;

        let password_file = self.config.read().await.password_file.clone();

        if !password_file.exists() {
            return Err("Password file does not exist".to_string());
        }

        let output = Command::new(passwd_bin)
            .arg("-D")
            .arg(&password_file)
            .arg(username)
            .output()
            .await
            .map_err(|e| format!("Failed to run mosquitto_passwd: {}", e))?;

        if !output.status.success() {
            return Err(format!(
                "mosquitto_passwd failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        info!("Deleted MQTT user: {}", username);
        Ok(())
    }
}

#[async_trait]
impl ManagedService for MosquittoService {
    async fn start(&mut self) -> Result<u32, String> {
        if self.child.is_some() {
            return Err("Mosquitto is already running".to_string());
        }

        let config_file = self.config.read().await.config_file.clone();
        if !config_file.exists() {
            return Err(format!(
                "Config file not found at {}. Call write_config() first.",
                config_file.display()
            ));
        }

        let mut child = Command::new(&self.binary_path)
            .args(["-c", &config_file.to_string_lossy()])
            .stderr(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to spawn mosquitto: {}", e))?;

        let pid = child.id().ok_or("Failed to get mosquitto PID")?;

        // Spawn a task to capture and log stderr
        if let Some(stderr) = child.stderr.take() {
            let reader = tokio::io::BufReader::new(stderr);
            tokio::spawn(async move {
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    info!(target: "mosquitto", "{}", line);
                }
            });
        }

        self.child = Some(child);
        Ok(pid)
    }

    async fn stop(&mut self) -> Result<(), String> {
        if let Some(mut child) = self.child.take() {
            // Send SIGTERM first for graceful shutdown
            let pid = child.id();
            if let Some(pid) = pid {
                unsafe {
                    libc::kill(pid as i32, libc::SIGTERM);
                }
            }

            // Wait briefly for graceful shutdown, then force kill
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                child.wait(),
            ).await {
                Ok(Ok(status)) => {
                    info!("Mosquitto exited with status: {}", status);
                }
                Ok(Err(e)) => {
                    error!("Error waiting for mosquitto: {}", e);
                }
                Err(_) => {
                    info!("Mosquitto didn't exit gracefully, killing");
                    let _ = child.kill().await;
                }
            }

            Ok(())
        } else {
            Err("Mosquitto is not running".to_string())
        }
    }

    async fn is_running(&mut self) -> bool {
        if let Some(child) = &mut self.child {
            // try_wait returns Ok(Some(status)) if exited, Ok(None) if still running
            match child.try_wait() {
                Ok(Some(_)) => {
                    self.child = None;
                    false
                }
                Ok(None) => true,
                Err(_) => false,
            }
        } else {
            false
        }
    }
}

/// Build and register `MosquittoService` onto the supervisor.
///
/// Returns whether mosquitto is available, so the caller can conditionally
/// start the broker and dependent services.
pub async fn register_onto(
    supervisor: &mut Supervisor,
    capabilities: &ServiceCapabilities,
    mqtt_config: Arc<RwLock<MqttBrokerageConfig>>,
) -> bool {
    let available = capabilities.is_available("mosquitto");

    if available {
        if let Err(e) = mqtt_config.read().await.write_config(false).await {
            error!("Failed to write mosquitto config: {}", e);
        }
    }

    let service = MosquittoService::new(
        capabilities.binary_path("mosquitto").unwrap_or("mosquitto").to_string(),
        capabilities.binary_path("mosquitto_passwd").map(String::from),
        mqtt_config,
    );

    supervisor.register(ServiceId::Mosquitto, Box::new(service), available);
    available
}

/// Start the Mosquitto broker via the supervisor.
pub async fn start_mosquitto_brokerage(
    mosquitto_available: bool,
    supervisor_handle: &SupervisorHandle,
) {
    if mosquitto_available {
        match supervisor_handle.start_service(ServiceId::Mosquitto).await {
            Ok(()) => {
                info!("Mosquitto broker started");
                // Brief delay to let the broker finish binding its port before
                // the MQTT client service tries to connect.
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            Err(e) => {
                error!("Failed to start Mosquitto: {}", e);
            }
        }
    } else {
        warn!("Mosquitto not installed — MQTT features disabled");
    }
}
