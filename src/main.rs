use axum::{routing::get, Router};
use rumqttc::{MqttOptions, AsyncClient};
use tower_http::{cors::CorsLayer, services::{ServeDir, ServeFile}};
use tracing::info;
use placenet_home::{database, routes, AppState};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    dotenvy::dotenv().ok();

    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite:assets/placenet.db".to_string());

    let frontend_dir = std::env::var("FRONTEND_DIR")
        .unwrap_or_else(|_| "frontend/dist".to_string());

    let host = std::env::var("SERVER_HOST")
        .unwrap_or_else(|_| "127.0.0.1".to_string());

    let port = std::env::var("SERVER_PORT")
        .unwrap_or_else(|_| "3000".to_string());

    let mqtt_host = std::env::var("MQTT_HOST")
        .unwrap_or_else(|_| "localhost".to_string());

    let mqtt_port: u16 = std::env::var("MQTT_PORT")
        .unwrap_or_else(|_| "1883".to_string())
        .parse()
        .unwrap_or(1883);

    let mqtt_client_id = std::env::var("MQTT_CLIENT_ID")
        .unwrap_or_else(|_| "placenet-home".to_string());

    info!("Using database: {}", db_url);
    info!("Serving frontend from: {}", frontend_dir);

    let db = database::create_pool(&db_url).await
        .expect("Failed to create database pool");

    let mut mqtt_options = MqttOptions::new(mqtt_client_id, mqtt_host, mqtt_port);
    mqtt_options.set_keep_alive(std::time::Duration::from_secs(30));

    let (mqtt, mut eventloop) = AsyncClient::new(mqtt_options, 10);

    tokio::spawn(async move {
        loop {
            match eventloop.poll().await {
                Ok(_) => {}
                Err(e) => {
                    tracing::error!("MQTT error: {}", e);
                }
            }
        }
    });

    let state = AppState { db, mqtt };

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

    let addr = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    info!("Server running at http://{}", addr);

    axum::serve(listener, app).await.unwrap();
}
