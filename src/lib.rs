pub mod database;
pub mod models;
pub mod mosquitto;
pub mod routes;
pub mod services;
pub mod supervisor;

use rumqttc::AsyncClient;
use sqlx::SqlitePool;

use crate::supervisor::SupervisorHandle;

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub mqtt: Option<AsyncClient>,
    pub supervisor: SupervisorHandle,
}
