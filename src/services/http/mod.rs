pub mod handshake;
pub mod manager;
pub mod routes;
pub mod tls;

use async_trait::async_trait;
use axum::{Router, routing::{get, post}};
use hyper::server::conn::http1;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_rustls::TlsAcceptor;
use tower_http::services::ServeDir;
use tower::ServiceExt;
use tracing::{error, info, warn};

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
            .route("/.well-known/placenet/register", post(http::routes::register_client))
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
            .map_err(|e| format!("Failed to bind TLS listener on {}: {}", addr, e))?;

        let local_addr = listener.local_addr()
            .map_err(|e| format!("Failed to get local address: {}", e))?;

        // Build TLS config from the live CA — happens before any connection is accepted.
        let tls_config = tls::build_tls_config(&self.state.ca).await?;
        let tls_acceptor = TlsAcceptor::from(tls_config);

        let app = self.create_app();

        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
        self.shutdown_tx = Some(shutdown_tx);

        tokio::spawn(async move {
            info!("HTTPS server listening on {}", local_addr);

            loop {
                // Poll both the listener and the shutdown signal.
                let accept = tokio::select! {
                    result = listener.accept() => result,
                    _ = &mut shutdown_rx => {
                        info!("HTTPS server shutting down");
                        break;
                    }
                };

                let (tcp_stream, peer_addr) = match accept {
                    Ok(pair) => pair,
                    Err(e) => {
                        warn!("TCP accept error: {}", e);
                        continue;
                    }
                };

                let tls_acceptor = tls_acceptor.clone();
                let app = app.clone();

                tokio::spawn(async move {
                    // ── TLS handshake ────────────────────────────────────────────
                    let tls_stream = match tls_acceptor.accept(tcp_stream).await {
                        Ok(s) => s,
                        Err(e) => {
                            warn!(%peer_addr, "TLS handshake failed: {}", e);
                            return;
                        }
                    };

                    info!(%peer_addr, "TLS handshake complete");

                    // ── Hand off to hyper / axum ─────────────────────────────────
                    let io = TokioIo::new(tls_stream);

                    // axum Router implements tower::Service<Request<Incoming>>.
                    // `ServiceExt::oneshot` consumes a single request — wrap in service_fn
                    // so hyper can drive multiple requests on the same connection.
                    let svc = hyper::service::service_fn(move |req| {
                        let app = app.clone();
                        async move { app.oneshot(req).await }
                    });

                    if let Err(e) = http1::Builder::new().serve_connection(io, svc).await {
                        // "connection reset" / "broken pipe" are normal on keep-alive close
                        let msg = e.to_string();
                        if !msg.contains("connection reset") && !msg.contains("broken pipe") {
                            error!(%peer_addr, "HTTP connection error: {}", e);
                        }
                    }
                });
            }
        });

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
