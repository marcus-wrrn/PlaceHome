use axum::{routing::get, Router};
use tower_http::{cors::CorsLayer, services::{ServeDir, ServeFile}};
use tracing::info;
use placenet_home::{routes, AppState};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    dotenvy::dotenv().ok();

    let db_path = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "assets/placenet.db".to_string());

    let frontend_dir = std::env::var("FRONTEND_DIR")
        .unwrap_or_else(|_| "frontend/dist".to_string());

    let host = std::env::var("SERVER_HOST")
        .unwrap_or_else(|_| "127.0.0.1".to_string());

    let port = std::env::var("SERVER_PORT")
        .unwrap_or_else(|_| "3000".to_string());

    info!("Using database: {}", db_path);
    info!("Serving frontend from: {}", frontend_dir);

    let state = AppState { db_path };

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
