use rcgen::{CertificateParams, KeyPair};
use sqlx::SqlitePool;

/// Create an in-memory SQLite pool with migrations applied.
pub async fn setup_db() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:").await.expect("in-memory db");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("migrations failed");
    pool
}

/// Generate a PEM-encoded CSR for use in tests.
pub fn generate_test_csr() -> String {
    let key = KeyPair::generate().expect("key generation failed");
    let params = CertificateParams::default();
    let csr = params.serialize_request(&key).expect("CSR generation failed");
    csr.pem().expect("CSR PEM serialization failed")
}
