use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};

pub async fn create_pool(db_url: &str) -> Result<SqlitePool, sqlx::Error> {
    let pool = SqlitePoolOptions::new()
        .connect(db_url)
        .await?;

    sqlx::query("PRAGMA journal_mode=WAL")
        .execute(&pool)
        .await?;

    sqlx::migrate!().run(&pool).await?;

    Ok(pool)
}
