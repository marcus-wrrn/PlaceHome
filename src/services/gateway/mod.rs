pub mod handshake;
pub mod manager;
pub mod tls;

use std::convert::Infallible;

use async_trait::async_trait;
use http_body_util::{BodyExt, Full, Limited, combinators::BoxBody};
use hyper::body::{Bytes, Incoming};
use hyper::{Request, Response};
use hyper::server::conn::http1;
use hyper_util::rt::TokioIo;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio_rustls::TlsAcceptor;
use tracing::{error, info, warn};

use crate::config::HttpConfig;
use crate::infra::ca::CaService;
use crate::supervisor::ManagedService;
use handshake::{DeviceInfo, MqttBrokerageInfo};

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

fn text_response(status: u16, msg: &'static str) -> Response<ProxyBody> {
    let body = Full::new(Bytes::from(msg))
        .map_err(|_: Infallible| unreachable!())
        .boxed();
    Response::builder().status(status).body(body).expect("valid response")
}

fn json_response(status: u16, body: Vec<u8>) -> Response<ProxyBody> {
    let body = Full::new(Bytes::from(body))
        .map_err(|_: Infallible| unreachable!())
        .boxed();
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(body)
        .expect("valid response")
}

/// `X-PlaceNet-Init: <version>` — device handshake: sign CSR, return cert + brokerage info.
async fn handle_device_init(state: &AppState, req: Request<Incoming>) -> Response<ProxyBody> {
    let version = match req.headers().get("x-placenet-init").and_then(|v| v.to_str().ok()) {
        Some(v) => v.to_owned(),
        None => return text_response(400, "Invalid X-PlaceNet-Init header"),
    };

    if version != SUPPORTED_VERSION {
        warn!("Unsupported PlaceNet init version: {}", version);
        return text_response(400, "Unsupported version");
    }

    let body_bytes = match Limited::new(req.into_body(), BODY_LIMIT).collect().await {
        Ok(b) => b.to_bytes(),
        Err(e) => {
            warn!("Failed to read device init body: {}", e);
            return text_response(400, "Failed to read request body");
        }
    };

    let device: DeviceInfo = match serde_json::from_slice(&body_bytes) {
        Ok(d) => d,
        Err(e) => {
            warn!("Invalid device init payload: {}", e);
            return text_response(422, "Invalid JSON body");
        }
    };

    info!(mdns_hostname = %device.mdns.hostname, address = %device.address, "Device handshake — signing CSR");

    let cert_pem = match state.ca.sign_csr(&device.mdns.hostname, &device.csr_pem).await {
        Ok(pem) => pem,
        Err(e) => {
            error!("Failed to sign device CSR: {}", e);
            return text_response(500, "Failed to sign device certificate");
        }
    };

    let resp = serde_json::json!({ "cert_pem": cert_pem, "brokerage": state.brokerage_info });
    json_response(200, serde_json::to_vec(&resp).unwrap())
}

#[derive(serde::Deserialize)]
struct ClientRegisterRequest {
    csr_pem: String,
}

/// `X-PlaceNet-Register: <version>` — client cert registration: sign CSR, return cert + CA cert.
async fn handle_client_register(state: &AppState, req: Request<Incoming>) -> Response<ProxyBody> {
    let body_bytes = match Limited::new(req.into_body(), BODY_LIMIT).collect().await {
        Ok(b) => b.to_bytes(),
        Err(e) => {
            warn!("Failed to read client register body: {}", e);
            return text_response(400, "Failed to read request body");
        }
    };

    let payload: ClientRegisterRequest = match serde_json::from_slice(&body_bytes) {
        Ok(p) => p,
        Err(e) => {
            warn!("Invalid client register payload: {}", e);
            return text_response(422, "Invalid JSON body");
        }
    };

    let client_id = uuid::Uuid::new_v4().to_string();

    let cert_pem = match state.ca.sign_csr(&client_id, &payload.csr_pem).await {
        Ok(pem) => pem,
        Err(e) => {
            error!(client_id, "Failed to sign client CSR: {}", e);
            return text_response(500, "Failed to sign client certificate");
        }
    };

    let ca_cert_pem = match state.ca.ca_cert_pem().await {
        Ok(pem) => pem,
        Err(e) => {
            error!("Failed to retrieve CA cert: {}", e);
            return text_response(500, "Failed to retrieve CA certificate");
        }
    };

    info!(client_id, "Client registered successfully");

    let resp = serde_json::json!({ "client_id": client_id, "cert_pem": cert_pem, "ca_cert_pem": ca_cert_pem });
    json_response(201, serde_json::to_vec(&resp).unwrap())
}

/// Dispatch a request: handle PlaceNet protocol requests locally, proxy everything else upstream.
async fn dispatch(state: AppState, req: Request<Incoming>) -> Result<Response<ProxyBody>, Infallible> {
    if req.headers().contains_key("x-placenet-init") {
        return Ok(handle_device_init(&state, req).await);
    }
    if req.headers().contains_key("x-placenet-register") {
        return Ok(handle_client_register(&state, req).await);
    }

    match try_forward(state.upstream_port, req).await {
        Ok(resp) => Ok(resp),
        Err(e) => {
            error!("Proxy error: {}", e);
            Ok(text_response(502, "Bad Gateway"))
        }
    }
}

async fn try_forward(upstream_port: u16, req: Request<Incoming>) -> Result<Response<ProxyBody>, BoxError> {
    let upstream = TcpStream::connect(("127.0.0.1", upstream_port)).await?;
    let io = TokioIo::new(upstream);

    let (mut sender, conn) = hyper::client::conn::http1::Builder::new()
        .handshake(io)
        .await?;

    tokio::spawn(async move {
        if let Err(e) = conn.await {
            error!("Upstream connection error: {}", e);
        }
    });

    let resp = sender.send_request(req).await?;
    Ok(resp.map(|b| b.boxed()))
}

async fn serve_connection(stream: TcpStream, state: AppState, peer_addr: std::net::SocketAddr) {
    let io = TokioIo::new(stream);
    let svc = hyper::service::service_fn(move |req| {
        let state = state.clone();
        async move { dispatch(state, req).await }
    });

    if let Err(e) = http1::Builder::new().serve_connection(io, svc).await {
        let msg = e.to_string();
        if !msg.contains("connection reset") && !msg.contains("broken pipe") {
            error!(%peer_addr, "HTTP connection error: {}", e);
        }
    }
}

async fn serve_tls_connection(
    stream: TcpStream,
    tls_acceptor: TlsAcceptor,
    state: AppState,
    peer_addr: std::net::SocketAddr,
) {
    let tls_stream = match tls_acceptor.accept(stream).await {
        Ok(s) => s,
        Err(e) => { warn!(%peer_addr, "TLS handshake failed: {}", e); return; }
    };

    info!(%peer_addr, "TLS handshake complete");

    let io = TokioIo::new(tls_stream);
    let svc = hyper::service::service_fn(move |req| {
        let state = state.clone();
        async move { dispatch(state, req).await }
    });

    if let Err(e) = http1::Builder::new().serve_connection(io, svc).await {
        let msg = e.to_string();
        if !msg.contains("connection reset") && !msg.contains("broken pipe") {
            error!(%peer_addr, "HTTPS connection error: {}", e);
        }
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

                    tokio::spawn(serve_tls_connection(tcp_stream, tls_acceptor.clone(), state.clone(), peer_addr));
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

                    tokio::spawn(serve_connection(tcp_stream, state.clone(), peer_addr));
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
