pub mod handshake;
pub mod manager;
pub mod routes;

use async_trait::async_trait;
use axum::{Router, routing::{get, post}};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tower_http::services::ServeDir;
use tracing::{error, info};

use crate::config::HttpConfig;
use crate::infra::ca::CaService;
use crate::services::http;
use crate::supervisor::ManagedService;
use handshake::MqttBrokerageInfo;

#[derive(Clone)]
pub struct AppState {
    pub brokerage_info: MqttBrokerageInfo,
    pub ca: CaService,
}

pub struct HttpService {
    config: HttpConfig,
    state: AppState,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl HttpService {
    pub fn new(config: HttpConfig, brokerage_info: MqttBrokerageInfo, ca: CaService) -> Self {
        Self {
            config,
            state: AppState { brokerage_info, ca },
            shutdown_tx: None,
        }
    }

    fn create_app(&self) -> Router {
        Router::new()
            .route("/", post(http::routes::init_device))
            .route("/health", get(http::routes::health))
            .nest_service("/static", ServeDir::new("static"))
            .with_state(self.state.clone())
    }
}

#[async_trait]
impl ManagedService for HttpService {
    async fn start(&mut self) -> Result<u32, String> {
        if self.shutdown_tx.is_some() {
            return Err("HttpService is already running".to_string());
        }

        let addr = format!("{}:{}", self.config.host, self.config.port);

        let listener = TcpListener::bind(&addr).await
            .map_err(|e| format!("Failed to bind HTTP listener on {}: {}", addr, e))?;

        let local_addr = listener.local_addr()
            .map_err(|e| format!("Failed to get local address: {}", e))?;

        let app = self.create_app();

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        self.shutdown_tx = Some(shutdown_tx);

        tokio::spawn(async move {
            info!("HTTP server listening on {}", local_addr);
            if let Err(e) = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
            {
                error!("HTTP server error: {}", e);
            }
            info!("HTTP server stopped");
        });

        // axum doesn't expose a child PID; return 0 as a sentinel
        Ok(0)
    }

    async fn stop(&mut self) -> Result<(), String> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
            Ok(())
        } else {
            Err("HttpService is not running".to_string())
        }
    }

    async fn is_running(&mut self) -> bool {
        self.shutdown_tx.is_some()
    }
}
