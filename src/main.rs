use std::sync::Arc;
use rustls;
use tokio::sync::RwLock;
use tracing::info;
use placenet_home::config::Config;
use placenet_home::services::mqtt_brokerage::manager::{register_onto as register_mqtt_broker, start_mosquitto_brokerage};
use placenet_home::services::mqtt_client::manager::{register_onto as register_mqtt_client, start_mqtt_client};
use placenet_home::infra::ca::manager::register as register_ca;
use placenet_home::services::gateway::manager::{register_onto as register_gateway, start_gateway};
use placenet_home::services::gateway::handshake::{build_brokerage_info, EnrichedRegistrationMessage};
use placenet_home::services::cloud_gateway::manager::{register_onto as register_cloud_gateway, start_cloud_gateway};
use placenet_home::services;
use placenet_home::services::mqtt_client;
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
    let ca_service = match register_ca(&ca_db_url).await {
        Ok(ca) => ca,
        Err(e) => {
            tracing::error!("Failed to initialise CA: {}", e);
            std::process::exit(1);
        }
    };

    // ── Register gateway service ─────────────────────────────────────
    let brokerage_info = build_brokerage_info(&config.mqtt_brokerage);
    register_gateway(&mut supervisor, config.http, brokerage_info, ca_service);

    // ── Register Mosquitto broker ────────────────────────────────────
    let mqtt_broker_config = Arc::new(RwLock::new(config.mqtt_brokerage));
    let broker_available = register_mqtt_broker(&mut supervisor, &capabilities, Arc::clone(&mqtt_broker_config)).await;

    // ── Register MQTT client ─────────────────────────────────────────
    let mqtt_handles = register_mqtt_client(&mut supervisor, config.mqtt_client, broker_available);
    let mqtt_handle = mqtt_handles.handle;
    let mut inbound_rx = mqtt_handles.inbound_rx;
    let _outbound_tx = mqtt_handles.outbound_tx;
    let mqtt_connected_rx = mqtt_handles.connected_rx;

    // ── Register cloud gateway client ────────────────────────────────
    let _cloud_gateway_handle = register_cloud_gateway(
        &mut supervisor,
        config.gateway_registration.server_url.clone(),
        config.gateway_registration.gateway_url.clone(),
    );

    let supervisor_handle = supervisor.spawn();

    // ── Start services ───────────────────────────────────────────────
    start_mosquitto_brokerage(broker_available, &supervisor_handle).await;
    start_mqtt_client(broker_available, &supervisor_handle).await;
    // Subscribe to Topics
    if broker_available {
        if mqtt_connected_rx.await.is_ok() {
            if let Err(e) = mqtt_handle.subscribe_all(&mqtt_client::required_subscriptions()).await {
                tracing::error!("Failed to subscribe to required topics: {}", e);
            }
        } else {
            tracing::error!("MQTT client disconnected before connection was established");
        }
    }

    start_gateway(&supervisor_handle).await;
    if _cloud_gateway_handle.is_some() {
        start_cloud_gateway(&supervisor_handle).await;
    }

    // ── Process inbound MQTT messages ────────────────────────────────
    let server_url = config.gateway_registration.server_url;
    let gateway_url = config.gateway_registration.gateway_url;
    tokio::spawn(async move {
        while let Some(msg) = inbound_rx.recv().await {
            info!(topic = %msg.topic, "Received MQTT message");

            if let Some(ref url) = gateway_url {
                // Parse the raw beacon payload, falling back to a string value
                // if it isn't valid JSON.
                let beacon_payload: serde_json::Value =
                    serde_json::from_slice(&msg.payload)
                        .unwrap_or_else(|_| {
                            serde_json::Value::String(
                                String::from_utf8_lossy(&msg.payload).into_owned(),
                            )
                        });

                let enriched = EnrichedRegistrationMessage {
                    beacon_payload,
                    server_url: server_url.clone(),
                    gateway_url: gateway_url.clone(),
                };

                match serde_json::to_string(&enriched) {
                    Ok(body) => {
                        let url = url.clone();
                        tokio::spawn(async move {
                            match placenet_home::services::peer::send_message(&url, &body).await {
                                Ok(()) => info!(peer = %url, "Forwarded registration to peer"),
                                Err(e) => tracing::error!(peer = %url, "Failed to forward to peer: {}", e),
                            }
                        });
                    }
                    Err(e) => tracing::error!("Failed to serialise enriched registration: {}", e),
                }
            }
        }
    });

    tokio::signal::ctrl_c().await.ok();
    info!("Shutting down");
}
