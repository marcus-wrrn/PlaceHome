use rumqttc::{AsyncClient, MqttOptions};
use tracing::{info, warn, error};
use placenet_home::config::Config;
use placenet_home::mosquitto::MosquittoService;
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
    let mosquitto_config_dir = config.config_dir.join("mosquitto");

    let mosquitto_service = MosquittoService::new(
        binary_path,
        password_binary,
        mosquitto_config_dir,
    );

    if mosquitto_available {
        if let Err(e) = mosquitto_service.write_config(config.mqtt.port, false).await {
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

                match connect_mqtt_client(&config.mqtt.client_id, config.mqtt.port).await {
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
