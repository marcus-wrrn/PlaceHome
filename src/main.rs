use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;
use placenet_home::config::Config;
use placenet_home::services::mqtt_brokerage::manager::{register_onto as register_mqtt_broker, start_mosquitto_brokerage};
use placenet_home::services::mqtt_client::manager::{register_onto as register_mqtt_client, start_mqtt_client};
use placenet_home::services;
use placenet_home::supervisor::Supervisor;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    dotenvy::dotenv().ok();

    let config = Config::from_env();

    // ── Service detection ────────────────────────────────────────────
    let required_binaries = ["mosquitto", "mosquitto_passwd"];
    let capabilities = services::detect_capabilities(&required_binaries).await;
    let mut supervisor = Supervisor::new();

    // ── Register Mosquitto broker ────────────────────────────────────
    let mqtt_broker_config = Arc::new(RwLock::new(config.mqtt_brokerage));
    let broker_available = register_mqtt_broker(&mut supervisor, &capabilities, Arc::clone(&mqtt_broker_config)).await;

    // ── Register MQTT client ─────────────────────────────────────────
    let mqtt_handles = register_mqtt_client(&mut supervisor, config.mqtt_client, broker_available);
    let _mqtt_handle = mqtt_handles.handle;
    let mut inbound_rx = mqtt_handles.inbound_rx;
    let _outbound_tx = mqtt_handles.outbound_tx;

    let supervisor_handle = supervisor.spawn();

    // Start services
    start_mosquitto_brokerage(broker_available, &supervisor_handle).await;
    start_mqtt_client(broker_available, &supervisor_handle).await;

    // ── Process inbound MQTT messages ────────────────────────────────
    tokio::spawn(async move {
        while let Some(msg) = inbound_rx.recv().await {
            info!(topic = %msg.topic, "Received MQTT message");
        }
    });

    tokio::signal::ctrl_c().await.ok();
    info!("Shutting down");
}
