use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, error};
use placenet_home::config::Config;
use placenet_home::services::mosquitto_brokerage_mngmt::{MosquittoService, start_mosquitto_brokerage};
use placenet_home::services::mosquitto_client::MqttClientConfig;
use placenet_home::services::mqtt_manager::MqttManager;
use placenet_home::services::{self, ServiceId};
use placenet_home::supervisor::Supervisor;

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
    
    // ── Build MQTT Client ────────────────────────────────────────────
    let mosquitto_available = capabilities.is_available("mosquitto");
    let mqtt_config = Arc::new(RwLock::new(config.mqtt));

    if mosquitto_available {
        if let Err(e) = mqtt_config.read().await.write_config(false).await {
            error!("Failed to write mosquitto config: {}", e);
        }
    }
   
    let mosquitto_service = MosquittoService::new(
        capabilities.binary_path("mosquitto").unwrap_or("mosquitto").to_string(), 
        capabilities.binary_path("mosquitto_passwd").map(String::from), 
        Arc::clone(&mqtt_config)
    );

    supervisor.register(
        ServiceId::Mosquitto,
        Box::new(mosquitto_service),
        mosquitto_available,
    );

    // ── Build MQTT client ────────────────────────────────────────────
    let (mqtt_client_id, mqtt_client_port) = {
        let cfg = mqtt_config.read().await;
        (cfg.client_id.clone(), cfg.port)
    };

    let mqtt_manager = MqttManager::new(MqttClientConfig {
        client_id: mqtt_client_id,
        host: "localhost".to_string(),
        port: mqtt_client_port,
    });

    let _mqtt_handle = mqtt_manager.handle.clone();
    let mut inbound_rx = mqtt_manager.inbound_rx;
    let _outbound_tx = mqtt_manager.outbound_tx.clone();

    supervisor.register(
        ServiceId::MqttClient,
        Box::new(mqtt_manager.service),
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

    // ── Process inbound MQTT messages ────────────────────────────────
    tokio::spawn(async move {
        while let Some(msg) = inbound_rx.recv().await {
            info!(topic = %msg.topic, "Received MQTT message");
        }
    });

    tokio::signal::ctrl_c().await.ok();
    info!("Shutting down");
}
