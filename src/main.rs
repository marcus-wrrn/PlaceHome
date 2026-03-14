use std::sync::Arc;
use rustls;
use tokio::sync::RwLock;
use tracing::info;
use placenet_home::config::Config;
use placenet_home::services::mqtt_brokerage::manager::{register_onto as register_mqtt_broker, start_mosquitto_brokerage};
use placenet_home::services::mqtt_client::manager::{register_onto as register_mqtt_client, start_mqtt_client};
use placenet_home::services::ca::manager::register as register_ca;
use placenet_home::services::http::manager::{register_onto as register_http_server, start_http};
use placenet_home::services::http::handshake::build_brokerage_info;
use placenet_home::services;
use placenet_home::supervisor::Supervisor;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install ring CryptoProvider");
    dotenvy::dotenv().ok();

    let config = Config::from_env();

    // ── Service detection ────────────────────────────────────────────
    let required_binaries = ["mosquitto", "mosquitto_passwd"];
    let capabilities = services::detect_capabilities(&required_binaries).await;
    let mut supervisor = Supervisor::new();

    // ── Initialise Certificate Authority ─────────────────────────────
    let ca_db_url = std::env::var("CA_DATABASE_URL")
        .unwrap_or_else(|_| "sqlite://placenet_ca.db".to_string());
    let _ca_service = match register_ca(&ca_db_url).await {
        Ok(ca) => ca,
        Err(e) => {
            tracing::error!("Failed to initialise CA: {}", e);
            std::process::exit(1);
        }
    };

    // ── Build MQTT brokerage info for device handshake response ──────
    let brokerage_info = build_brokerage_info(&config.mqtt_brokerage);

    // ── Register HTTP server ─────────────────────────────────────────
    register_http_server(&mut supervisor, config.http, brokerage_info);

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
    start_http(&supervisor_handle).await;

    // ── Process inbound MQTT messages ────────────────────────────────
    tokio::spawn(async move {
        while let Some(msg) = inbound_rx.recv().await {
            info!(topic = %msg.topic, "Received MQTT message");
        }
    });

    tokio::signal::ctrl_c().await.ok();
    info!("Shutting down");
}
