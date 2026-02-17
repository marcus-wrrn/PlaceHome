pub mod database;
pub mod models;
pub mod routes;

#[derive(Clone)]
pub struct AppState {
    pub db_path: String,
}
