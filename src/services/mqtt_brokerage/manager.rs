use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn, error};

use crate::config::MqttBrokerageConfig;
use crate::supervisor::{Supervisor, SupervisorHandle};
use crate::services::{ServiceCapabilities, ServiceId};
use super::{MosquittoBrokerageService, MqttBrokerageHandle};

/// Build and register `MosquittoBrokerageService` onto the supervisor.
///
/// Returns whether mosquitto is available and, when it is, a handle for runtime
/// provisioning (e.g. registering device cert identities during the handshake).
pub async fn register_onto(
    supervisor: &mut Supervisor,
    capabilities: &ServiceCapabilities,
    mqtt_config: Arc<RwLock<MqttBrokerageConfig>>,
) -> (bool, Option<MqttBrokerageHandle>) {
    let available = capabilities.is_available("mosquitto");

    let service = MosquittoBrokerageService::new(
        capabilities.binary_path("mosquitto").unwrap_or("mosquitto").to_string(),
        capabilities.binary_path("mosquitto_passwd").map(String::from),
        Arc::clone(&mqtt_config),
    );

    let handle = if available { Some(service.handle()) } else { None };

    if available {
        let config = mqtt_config.read().await;
        if let Err(e) = config.write_config().await {
            error!("Failed to write mosquitto config: {}", e);
        }

        // In non-TLS mode the broker uses password-file auth — seed the initial admin user.
        // In TLS mode, cert-based auth is used and no password file is needed.
        if !config.tls_enabled {
            let username = config.username.clone();
            let password = config.password.clone();
            drop(config);

            if let Err(e) = service.set_password(&username, &password).await {
                error!("Failed to set MQTT user password: {}", e);
            } else {
                info!("MQTT user '{}' configured", username);
            }
        }
    }

    supervisor.register(ServiceId::Mosquitto, Box::new(service), available);
    (available, handle)
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
