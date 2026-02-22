use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, error};
use placenet_home::config::Config;
use placenet_home::services::mosquitto_brokerage_mngmt::{register_onto as register_mosquitto, start_mosquitto_brokerage};
use placenet_home::services::mqtt_manager::register_onto as register_mqtt_client;
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

    // ── Register Mosquitto broker ────────────────────────────────────
    let mqtt_broker_config = Arc::new(RwLock::new(config.mqtt_brokerage));
    let mosquitto_available = register_mosquitto(&mut supervisor, &capabilities, Arc::clone(&mqtt_broker_config)).await;

    // ── Register MQTT client ─────────────────────────────────────────
    let mqtt_handles = register_mqtt_client(&mut supervisor, config.mqtt_client, mosquitto_available);
    let _mqtt_handle = mqtt_handles.handle;
    let mut inbound_rx = mqtt_handles.inbound_rx;
    let _outbound_tx = mqtt_handles.outbound_tx;

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
