use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, error};
use placenet_home::config::{Config};
use placenet_home::services::mosquitto_brokerage_mngmt::{MosquittoService, start_mosquitto_brokerage};
use placenet_home::services::mosquitto_client::{MqttClientConfig, MqttClientService};
use placenet_home::services::{self, ServiceId};
use placenet_home::supervisor::{Supervisor};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    dotenvy::dotenv().ok();

    let config = Config::from_env();

    // ── Service detection ────────────────────────────────────────────
    let required_binaries = ["mosquitto", "mosquitto_passwd"];
    let capabilities = services::detect_capabilities(&required_binaries).await;

    // ── Build supervisor ─────────────────────────────────────────────
    let mut supervisor = Supervisor::new();

    let mosquitto_available = capabilities.is_available("mosquitto");

    let binary_path = capabilities
            .binary_path("mosquitto")
            .unwrap_or("mosquitto")
            .to_string();
    let password_binary = capabilities.binary_path("mosquitto_passwd").map(String::from);

    let mqtt_config = Arc::new(RwLock::new(config.mqtt));

    let mosquitto_service = MosquittoService::new(
        binary_path,
        password_binary,
        Arc::clone(&mqtt_config),
    );

    if mosquitto_available {
        if let Err(e) = mqtt_config.read().await.write_config(false).await {
            error!("Failed to write mosquitto config: {}", e);
        }
    }

    supervisor.register(
        ServiceId::Mosquitto,
        Box::new(mosquitto_service),
        mosquitto_available,
    );

    // ── Build MQTT client service ────────────────────────────────────
    let (mqtt_client_config_id, mqtt_client_port) = {
        let cfg = mqtt_config.read().await;
        (cfg.client_id.clone(), cfg.port)
    };
    let mqtt_client_service = MqttClientService::new(MqttClientConfig {
        client_id: mqtt_client_config_id,
        host: "localhost".to_string(),
        port: mqtt_client_port,
    });
    let _mqtt_client_handle = mqtt_client_service.handle();

    supervisor.register(
        ServiceId::MqttClient,
        Box::new(mqtt_client_service),
        mosquitto_available,
    );

    let supervisor_handle = supervisor.spawn();

    // ── Start Mosquitto broker ───────────────────────────────────────
    start_mosquitto_brokerage(mosquitto_available, &supervisor_handle).await;

    // ── Start MQTT client ────────────────────────────────────────────
    if mosquitto_available {
        if let Err(e) = supervisor_handle.start_service(ServiceId::MqttClient).await {
            error!("Failed to start MQTT client service: {}", e);
        }
    }

    // Keep running until interrupted
    tokio::signal::ctrl_c().await.ok();
    info!("Shutting down");
}
