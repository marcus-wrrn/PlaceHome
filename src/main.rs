use axum::{routing::get, Router};
use rumqttc::{AsyncClient, MqttOptions};
use tower_http::{
    cors::CorsLayer,
    services::{ServeDir, ServeFile},
};
use tracing::{info, warn, error};

use placenet_home::mosquitto::MosquittoService;
use placenet_home::services::{self, ServiceId};
use placenet_home::supervisor::Supervisor;
use placenet_home::{database, routes, AppState};

use std::path::PathBuf;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    dotenvy::dotenv().ok();

    // ── Environment config ───────────────────────────────────────────
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite:assets/placenet.db".to_string());

    let frontend_dir = std::env::var("FRONTEND_DIR")
        .unwrap_or_else(|_| "frontend/dist".to_string());

    let host = std::env::var("SERVER_HOST")
        .unwrap_or_else(|_| "127.0.0.1".to_string());

    let port = std::env::var("SERVER_PORT")
        .unwrap_or_else(|_| "3000".to_string());

    let mqtt_port: u16 = std::env::var("MQTT_PORT")
        .unwrap_or_else(|_| "1883".to_string())
        .parse()
        .unwrap_or(1883);

    let mqtt_client_id = std::env::var("MQTT_CLIENT_ID")
        .unwrap_or_else(|_| "placenet-home".to_string());

    let config_dir = std::env::var("PLACENET_CONFIG_DIR")
        .unwrap_or_else(|_| "config".to_string());

    info!("Using database: {}", db_url);
    info!("Serving frontend from: {}", frontend_dir);

    // ── Database ─────────────────────────────────────────────────────
    let db = database::create_pool(&db_url)
        .await
        .expect("Failed to create database pool");

    // ── Service detection ────────────────────────────────────────────
    let capabilities = services::detect_capabilities().await;

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

    // Write default config if mosquitto is available
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
    let mqtt_client = if mosquitto_available {
        match supervisor_handle.start_service(ServiceId::Mosquitto).await {
            Ok(()) => {
                info!("Mosquitto broker started, connecting MQTT client...");

                // Give Mosquitto a moment to bind its port
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

    // ── App state ────────────────────────────────────────────────────
    let state = AppState {
        db,
        mqtt: mqtt_client,
        supervisor: supervisor_handle,
    };

    // ── HTTP routes ──────────────────────────────────────────────────
    let api_routes = Router::new()
        .route("/health", get(routes::health));

    let index_path = format!("{}/index.html", frontend_dir);
    let spa_fallback = ServeDir::new(&frontend_dir)
        .not_found_service(ServeFile::new(index_path));

    let app = Router::new()
        .nest("/api", api_routes)
        .fallback_service(spa_fallback)
        .layer(CorsLayer::permissive())
        .with_state(state);

    // ── Start HTTP server ────────────────────────────────────────────
    let addr = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    info!("Server running at http://{}", addr);

    axum::serve(listener, app).await.unwrap();
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
