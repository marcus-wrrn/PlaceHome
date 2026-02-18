pub mod database;
pub mod models;
pub mod routes;

use rumqttc::AsyncClient;
use sqlx::SqlitePool;

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub mqtt: AsyncClient,
}
