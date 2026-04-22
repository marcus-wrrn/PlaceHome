pub mod manager;

use std::sync::Arc;
use async_trait::async_trait;
use tokio::process::{Child, Command};
use tokio::io::AsyncBufReadExt;
use tokio::sync::RwLock;
use tracing::{info, error};

use crate::config::MqttBrokerageConfig;
use crate::supervisor::ManagedService;

/// A cloneable handle for interacting with the MQTT brokerage at runtime.
#[derive(Clone)]
pub struct MqttBrokerageHandle {
    config: Arc<RwLock<MqttBrokerageConfig>>,
}

impl MqttBrokerageHandle {
    /// Write the signed device certificate to `{certs_dir}/{hostname}.crt` to record the
    /// device's MQTT identity. Mosquitto with `require_certificate true` accepts any CA-signed
    /// cert; this file is a local record used for future dynamic-security provisioning.
    pub async fn register_cert_identity(&self, hostname: &str, cert_pem: &str) -> Result<(), String> {
        let certs_dir = {
            let config = self.config.read().await;
            config.cafile
                .parent()
                .ok_or_else(|| "cafile has no parent directory".to_string())?
                .to_path_buf()
        };
        let cert_path = certs_dir.join(format!("{}.crt", hostname));
        tokio::fs::write(&cert_path, cert_pem).await
            .map_err(|e| format!("Failed to write device cert '{}': {}", cert_path.display(), e))?;
        info!(hostname, cert_path = %cert_path.display(), "Device MQTT cert identity registered");
        Ok(())
    }
}

/// Manages a Mosquitto MQTT broker child process
pub struct MosquittoBrokerageService {
    binary_path: String,
    password_binary: Option<String>,
    config: Arc<RwLock<MqttBrokerageConfig>>,
    child: Option<Child>,
}

impl MosquittoBrokerageService {
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

    pub fn handle(&self) -> MqttBrokerageHandle {
        MqttBrokerageHandle { config: Arc::clone(&self.config) }
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
impl ManagedService for MosquittoBrokerageService {
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

        let mut cmd = Command::new(&self.binary_path);
        cmd.args(["-c", &config_file.to_string_lossy()])
            .stderr(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .kill_on_drop(true);

        // On Linux: ask the kernel to send SIGTERM to mosquitto if this process dies,
        // even if killed with SIGKILL (the only reliable way to handle abnormal termination).
        #[cfg(target_os = "linux")]
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(
                    libc::PR_SET_PDEATHSIG,
                    libc::SIGTERM as libc::c_ulong,
                    0, 0, 0,
                );
                Ok(())
            });
        }

        let mut child = cmd.spawn()
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
