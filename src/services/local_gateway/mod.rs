pub mod handshake;
pub mod manager;
pub mod tls;
mod handlers;
mod headers;
mod proxy;
mod requests;
mod response;

use async_trait::async_trait;
use http_body_util::combinators::BoxBody;
use hyper::body::Bytes;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_rustls::TlsAcceptor;
use tracing::{info, warn};

use crate::config::HttpConfig;
use crate::infra::ca::CaService;
use crate::supervisor::ManagedService;
use handshake::MqttBrokerageInfo;

const SUPPORTED_VERSION: &str = "0.0.1";
const BODY_LIMIT: usize = 64 * 1024;

type BoxError = Box<dyn std::error::Error + Send + Sync>;
type ProxyBody = BoxBody<Bytes, hyper::Error>;

#[derive(Clone)]
struct AppState {
    ca: CaService,
    brokerage_info: MqttBrokerageInfo,
    upstream_port: u16,
}

pub struct GatewayService {
    config: HttpConfig,
    ca: CaService,
    brokerage_info: MqttBrokerageInfo,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl GatewayService {
    pub fn new(config: HttpConfig, ca: CaService, brokerage_info: MqttBrokerageInfo) -> Self {
        Self { config, ca, brokerage_info, shutdown_tx: None }
    }
}

#[async_trait]
impl ManagedService for GatewayService {
    async fn start(&mut self) -> Result<u32, String> {
        if self.shutdown_tx.is_some() {
            return Err("GatewayService is already running".to_string());
        }

        let addr = format!("{}:{}", self.config.host, self.config.port);
        let state = AppState {
            ca: self.ca.clone(),
            brokerage_info: self.brokerage_info.clone(),
            upstream_port: self.config.upstream_port,
        };

        let listener = TcpListener::bind(&addr).await
            .map_err(|e| format!("Failed to bind listener on {}: {}", addr, e))?;

        let local_addr = listener.local_addr()
            .map_err(|e| format!("Failed to get local address: {}", e))?;

        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
        self.shutdown_tx = Some(shutdown_tx);

        if self.config.tls_enabled {
            let tls_config = tls::build_tls_config(&self.ca).await?;
            let tls_acceptor = TlsAcceptor::from(tls_config);

            tokio::spawn(async move {
                info!("HTTPS service listening on {} → upstream :{}", local_addr, state.upstream_port);

                loop {
                    let (tcp_stream, peer_addr) = tokio::select! {
                        result = listener.accept() => match result {
                            Ok(pair) => pair,
                            Err(e) => { warn!("TCP accept error: {}", e); continue; }
                        },
                        _ = &mut shutdown_rx => { info!("HTTPS service shutting down"); break; }
                    };

                    tokio::spawn(proxy::serve_tls_connection(tcp_stream, tls_acceptor.clone(), state.clone(), peer_addr));
                }
            });
        } else {
            tokio::spawn(async move {
                info!("HTTP service listening on {} → upstream :{} (TLS disabled)", local_addr, state.upstream_port);

                loop {
                    let (tcp_stream, peer_addr) = tokio::select! {
                        result = listener.accept() => match result {
                            Ok(pair) => pair,
                            Err(e) => { warn!("TCP accept error: {}", e); continue; }
                        },
                        _ = &mut shutdown_rx => { info!("HTTP service shutting down"); break; }
                    };

                    tokio::spawn(proxy::serve_connection(tcp_stream, state.clone(), peer_addr));
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
            Err("GatewayService is not running".to_string())
        }
    }

    async fn is_running(&mut self) -> bool {
        self.shutdown_tx.is_some()
    }
}
