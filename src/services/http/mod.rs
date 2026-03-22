pub mod handshake;
pub mod manager;
pub mod routes;
pub mod tls;

use async_trait::async_trait;
use axum::{Router, routing::{get, post}};
use hyper::server::conn::http1;
use hyper_util::rt::TokioIo;
use tokio::net::{TcpListener, TcpStream};
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
            .route("/peer/message", post(http::routes::peer_message))
            .route("/.well-known/placenet/register", post(http::routes::register_client))
            .nest_service("/static", ServeDir::new("static"))
            .with_state(self.state.clone())
    }
}

async fn serve_connection(stream: TcpStream, app: Router, peer_addr: std::net::SocketAddr) {
    let io = TokioIo::new(stream);
    let svc = hyper::service::service_fn(move |req| {
        let app = app.clone();
        async move { app.oneshot(req).await }
    });

    if let Err(e) = http1::Builder::new().serve_connection(io, svc).await {
        let msg = e.to_string();
        if !msg.contains("connection reset") && !msg.contains("broken pipe") {
            error!(%peer_addr, "HTTP connection error: {}", e);
        }
    }
}

async fn serve_tls_connection(stream: TcpStream, tls_acceptor: TlsAcceptor, app: Router, peer_addr: std::net::SocketAddr) {
    let tls_stream = match tls_acceptor.accept(stream).await {
        Ok(s) => s,
        Err(e) => { warn!(%peer_addr, "TLS handshake failed: {}", e); return; }
    };

    info!(%peer_addr, "TLS handshake complete");

    let io = TokioIo::new(tls_stream);
    let svc = hyper::service::service_fn(move |req| {
        let app = app.clone();
        async move { app.oneshot(req).await }
    });

    if let Err(e) = http1::Builder::new().serve_connection(io, svc).await {
        let msg = e.to_string();
        if !msg.contains("connection reset") && !msg.contains("broken pipe") {
            error!(%peer_addr, "HTTP connection error: {}", e);
        }
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
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
        self.shutdown_tx = Some(shutdown_tx);

        if self.config.tls_enabled {
            // Build TLS config from the live CA — happens before any connection is accepted.
            let tls_config = tls::build_tls_config(&self.state.ca).await?;
            let tls_acceptor = TlsAcceptor::from(tls_config);

            tokio::spawn(async move {
                info!("HTTPS server listening on {}", local_addr);

                loop {
                    let (tcp_stream, peer_addr) = tokio::select! {
                        result = listener.accept() => match result {
                            Ok(pair) => pair,
                            Err(e) => { warn!("TCP accept error: {}", e); continue; }
                        },
                        _ = &mut shutdown_rx => { info!("HTTPS server shutting down"); break; }
                    };

                    tokio::spawn(serve_tls_connection(tcp_stream, tls_acceptor.clone(), app.clone(), peer_addr));
                }
            });
        } else {
            tokio::spawn(async move {
                info!("HTTP server listening on {} (TLS disabled)", local_addr);

                loop {
                    let (tcp_stream, peer_addr) = tokio::select! {
                        result = listener.accept() => match result {
                            Ok(pair) => pair,
                            Err(e) => { warn!("TCP accept error: {}", e); continue; }
                        },
                        _ = &mut shutdown_rx => { info!("HTTP server shutting down"); break; }
                    };

                    tokio::spawn(serve_connection(tcp_stream, app.clone(), peer_addr));
                }
            });
        }

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
