use http_body_util::{BodyExt, Limited};
use hyper::body::Incoming;
use hyper::{Request, Response};
use tracing::{debug, error, info, warn};

use super::headers::HEADER_INIT;
use super::response::{json_response, text_response};
use super::{AppState, ProxyBody, BODY_LIMIT, SUPPORTED_VERSION};

#[derive(serde::Deserialize)]
pub(super) struct ClientRegisterRequest {
    pub(super) csr_pem: String,
}

/// `X-PlaceNet-Health` — health check: returns 200 OK.
pub(super) async fn handle_health(_req: Request<Incoming>) -> Response<ProxyBody> {
    text_response(200, "OK")
}

/// `X-PlaceNet-Init: <version>` — device handshake: sign CSR, return cert + brokerage info.
pub(super) async fn handle_device_init(state: &AppState, req: Request<Incoming>) -> Response<ProxyBody> {
    let version = match req.headers().get(HEADER_INIT).and_then(|v| v.to_str().ok()) {
        Some(v) => v.to_owned(),
        None => return text_response(400, "Invalid X-PlaceNet-Init header"),
    };

    if version != SUPPORTED_VERSION {
        warn!("Unsupported PlaceNet init version: {}", version);
        return text_response(400, "Unsupported version");
    }

    // Extract Host header before consuming req with into_body().
    let broker_host = req
        .headers()
        .get(hyper::header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(|h| {
            // Strip port: "192.168.2.39:8080" → "192.168.2.39"
            match h.rfind(':') {
                Some(i) if !h.starts_with('[') => h[..i].to_string(),
                _ => h.to_string(),
            }
        })
        .unwrap_or_else(|| "localhost".to_string());

    let body_bytes = match Limited::new(req.into_body(), BODY_LIMIT).collect().await {
        Ok(b) => b.to_bytes(),
        Err(e) => {
            warn!("Failed to read device init body: {}", e);
            return text_response(400, "Failed to read request body");
        }
    };

    let device: super::handshake::DeviceInfo = match serde_json::from_slice(&body_bytes) {
        Ok(d) => d,
        Err(e) => {
            warn!("Invalid device init payload: {}", e);
            return text_response(422, "Invalid JSON body");
        }
    };

    info!(
        mdns_hostname = %device.mdns.hostname,
        mdns_port = device.mdns.port,
        address = %device.address,
        "Device handshake — signing CSR",
    );
    debug!(
        mdns_hostname = %device.mdns.hostname,
        address = %device.address,
        csr_pem = %device.csr_pem,
        "Device init request payload",
    );

    let cert_pem = match state.ca.sign_csr(&device.mdns.hostname, &device.csr_pem).await {
        Ok(pem) => pem,
        Err(e) => {
            error!("Failed to sign device CSR: {}", e);
            return text_response(500, "Failed to sign device certificate");
        }
    };

    let mut brokerage = state.brokerage_info.clone();
    brokerage.address = broker_host;

    let resp = serde_json::json!({ "cert_pem": cert_pem, "brokerage": brokerage });
    debug!(
        mdns_hostname = %device.mdns.hostname,
        broker_address = %brokerage.address,
        broker_port = brokerage.port,
        cert_pem = %cert_pem,
        brokerage_info = %serde_json::to_string(&brokerage).unwrap_or_default(),
        "Device init response payload",
    );
    json_response(200, serde_json::to_vec(&resp).unwrap())
}

/// `X-PlaceNet-Register: <version>` — client cert registration: sign CSR, return cert + CA cert.
pub(super) async fn handle_client_register(state: &AppState, req: Request<Incoming>) -> Response<ProxyBody> {
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

    info!(client_id, "Client register — signing CSR");
    debug!(client_id, csr_pem = %payload.csr_pem, "Client register request payload");

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
    debug!(
        client_id,
        cert_pem = %cert_pem,
        ca_cert_pem = %ca_cert_pem,
        "Client register response payload",
    );

    let resp = serde_json::json!({ "client_id": client_id, "cert_pem": cert_pem, "ca_cert_pem": ca_cert_pem });
    json_response(201, serde_json::to_vec(&resp).unwrap())
}
