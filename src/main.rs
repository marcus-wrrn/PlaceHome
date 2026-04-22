use std::sync::Arc;
use rustls;
use tokio::sync::RwLock;
use tracing::info;
use placenet_home::config::Config;
use placenet_home::services::mqtt_brokerage::manager::{register_onto as register_mqtt_broker, start_mosquitto_brokerage};
use placenet_home::services::mqtt_brokerage::provision_broker_cert;
use placenet_home::services::mqtt_client::manager::{register_onto as register_mqtt_client, start_mqtt_client};
use placenet_home::services::mqtt_client::provision_node_identity;
use placenet_home::infra::ca::manager::register as register_ca;
use placenet_home::services::local_gateway::manager::{register_onto as register_gateway, start_gateway};
use placenet_home::services::local_gateway::handshake::build_brokerage_info;
use placenet_home::services::cloud_gateway::{connect_to_gateway, messages::GatewayMessage};
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

    // ── Build gateway brokerage info ─────────────────────────────────
    let ca_cert_pem = match ca_service.ca_cert_pem().await {
        Ok(pem) => pem,
        Err(e) => {
            tracing::error!("Failed to retrieve CA cert PEM: {}", e);
            std::process::exit(1);
        }
    };
    let brokerage_info = build_brokerage_info(&config.mqtt_brokerage, ca_cert_pem);

    // ── Provision broker TLS cert ────────────────────────────────────
    if config.mqtt_brokerage.tls_enabled {
        if let Err(e) = provision_broker_cert(&ca_service, &config.mqtt_brokerage).await {
            tracing::error!("{}", e);
            std::process::exit(1);
        }
    }

    // ── Register Mosquitto broker ────────────────────────────────────
    let mqtt_broker_config = Arc::new(RwLock::new(config.mqtt_brokerage));
    let (broker_available, brokerage_handle) = register_mqtt_broker(&mut supervisor, &capabilities, Arc::clone(&mqtt_broker_config)).await;

    // ── Register gateway service ─────────────────────────────────────
    register_gateway(&mut supervisor, config.http, brokerage_info, brokerage_handle, ca_service.clone());

    // ── Provision node identity for MQTT mutual TLS ──────────────────
    if config.mqtt_client.tls_enabled {
        if let Err(e) = provision_node_identity(&ca_service, &config.mqtt_client).await {
            tracing::error!("{}", e);
            std::process::exit(1);
        }
    }

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
    let own_gateway_url = config.gateway_registration.gateway_url;
    tokio::spawn(async move {
        // Seed the known-gateways set with this server's own configured gateway so
        // beacons advertising the same gateway don't trigger a redundant connection.
        let mut known_gateways = std::collections::HashSet::new();
        if let Some(ref url) = own_gateway_url {
            known_gateways.insert(url.clone());
        }

        // Keep (handle, shutdown_tx) pairs alive so dynamic connections persist.
        let mut dynamic_connections = Vec::new();

        while let Some(msg) = inbound_rx.recv().await {
            info!(topic = %msg.topic, "Received MQTT message");

            // Parse the raw beacon payload, falling back to a string value if it
            // isn't valid JSON.
            let beacon_payload: serde_json::Value =
                serde_json::from_slice(&msg.payload)
                    .unwrap_or_else(|_| {
                        serde_json::Value::String(
                            String::from_utf8_lossy(&msg.payload).into_owned(),
                        )
                    });

            // Extract the gateway_url advertised by the beacon, if present.
            let beacon_gateway_url = beacon_payload
                .as_object()
                .and_then(|obj| obj.get("gateway_url"))
                .and_then(|v| v.as_str())
                .map(str::to_owned);

            if let Some(gw_url) = beacon_gateway_url {
                if known_gateways.contains(&gw_url) {
                    info!(gateway = %gw_url, "Beacon's gateway already connected, skipping");
                } else {
                    info!(gateway = %gw_url, "Beacon advertises new gateway — registering");
                    known_gateways.insert(gw_url.clone());
                    let (handle, shutdown_tx) = connect_to_gateway(server_url.clone(), gw_url.clone());
                    // Queue the beacon payload immediately; the connection task will
                    // deliver it after the Register frame is sent.
                    let registration = GatewayMessage::BeaconRegistration {
                        server_url: server_url.clone(),
                        payload: beacon_payload,
                    };
                    if let Err(e) = handle.send(registration).await {
                        tracing::error!(gateway = %gw_url, "Failed to queue beacon payload: {}", e);
                    }
                    dynamic_connections.push((handle, shutdown_tx));
                }
            }
        }
    });

    tokio::signal::ctrl_c().await.ok();
    info!("Shutting down");
}
