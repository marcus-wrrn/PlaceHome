use rumqttc::{AsyncClient, MqttOptions};
use tracing::{info, warn, error};

use placenet_home::mosquitto::MosquittoService;
use placenet_home::services::{self, ServiceId};
use placenet_home::supervisor::Supervisor;

use std::path::PathBuf;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    dotenvy::dotenv().ok();

    // ── Environment config ───────────────────────────────────────────
    let mqtt_port: u16 = std::env::var("MQTT_PORT")
        .unwrap_or_else(|_| "1883".to_string())
        .parse()
        .unwrap_or(1883);

    let mqtt_client_id = std::env::var("MQTT_CLIENT_ID")
        .unwrap_or_else(|_| "placenet-home".to_string());

    let config_dir = std::env::var("PLACENET_CONFIG_DIR")
        .unwrap_or_else(|_| "config".to_string());

    // ── Service detection ────────────────────────────────────────────
    let required_binaries = ["mosquitto", "mosquitto_passwd"];
    let capabilities = services::detect_capabilities(&required_binaries).await;

    // ── Build supervisor ─────────────────────────────────────────────
    let mut supervisor = Supervisor::new();

    let mosquitto_available = capabilities.is_available("mosquitto");
    let mosquitto_config_dir = PathBuf::from(&config_dir).join("mosquitto");

    let mosquitto_service = MosquittoService::new(
        capabilities
            .binary_path("mosquitto")
            .unwrap_or("mosquitto")
            .to_string(),
        capabilities.binary_path("mosquitto_passwd").map(String::from),
        mosquitto_config_dir,
    );

    if mosquitto_available {
        if let Err(e) = mosquitto_service.write_config(mqtt_port, false).await {
            error!("Failed to write mosquitto config: {}", e);
        }
    }

    supervisor.register(
        ServiceId::Mosquitto,
        Box::new(mosquitto_service),
        mosquitto_available,
    );

    let supervisor_handle = supervisor.spawn();

    // ── Start Mosquitto broker ───────────────────────────────────────
    let _mqtt_client: Option<AsyncClient> = if mosquitto_available {
        match supervisor_handle.start_service(ServiceId::Mosquitto).await {
            Ok(()) => {
                info!("Mosquitto broker started, connecting MQTT client...");

                tokio::time::sleep(std::time::Duration::from_millis(500)).await;

                match connect_mqtt_client(&mqtt_client_id, mqtt_port).await {
                    Ok(client) => Some(client),
                    Err(e) => {
                        error!("Failed to connect MQTT client: {}", e);
                        None
                    }
                }
            }
            Err(e) => {
                error!("Failed to start Mosquitto: {}", e);
                None
            }
        }
    } else {
        warn!("Mosquitto not installed — MQTT features disabled");
        None
    };

    // Keep running until interrupted
    tokio::signal::ctrl_c().await.ok();
    info!("Shutting down");
}

/// Connect the rumqttc async client to the local broker
async fn connect_mqtt_client(
    client_id: &str,
    port: u16,
) -> Result<AsyncClient, String> {
    let mut opts = MqttOptions::new(client_id, "localhost", port);
    opts.set_keep_alive(std::time::Duration::from_secs(30));

    let (client, mut eventloop) = AsyncClient::new(opts, 10);

    tokio::spawn(async move {
        loop {
            match eventloop.poll().await {
                Ok(_) => {}
                Err(e) => {
                    tracing::error!("MQTT client error: {}", e);
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            }
        }
    });

    Ok(client)
}
