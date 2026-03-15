use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::str::FromStr;
use super::CaService;

/// Construct and initialise the CA service.
///
/// Opens (or creates) the SQLite database at `db_url`, runs migrations,
/// and loads or generates the root CA.
pub async fn register(db_url: &str) -> Result<CaService, String> {
    let options = SqliteConnectOptions::from_str(db_url)
        .map_err(|e| format!("Invalid database URL: {}", e))?
        .create_if_missing(true);

    let pool = SqlitePoolOptions::new()
        .connect_with(options)
        .await
        .map_err(|e| format!("Failed to open CA database: {}", e))?;

    let service = CaService::new(pool);
    service.init().await?;
    Ok(service)
}
