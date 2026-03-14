pub mod manager;
pub mod operations;

use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

use operations::CaState;

/// Holds the loaded/generated CA state, shared across the application.
#[derive(Clone)]
pub struct CaService {
    pub state: Arc<RwLock<Option<CaState>>>,
}

impl CaService {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(None)),
        }
    }

    /// Initialise the CA: load from disk if certificates exist, otherwise
    /// generate a new root CA and persist it.
    pub async fn init(&self) -> Result<(), String> {
        let ca_state = operations::load_or_generate_ca().await?;
        info!("CA ready (CN={})", ca_state.subject_cn);
        *self.state.write().await = Some(ca_state);
        Ok(())
    }

    /// Sign a PEM-encoded CSR and return the signed certificate as PEM.
    pub async fn sign_csr(&self, csr_pem: &str) -> Result<String, String> {
        let guard = self.state.read().await;
        let ca = guard.as_ref().ok_or("CA not initialised")?;
        operations::sign_csr(ca, csr_pem)
    }
}
