use axum::{
    Json,
    body::Body,
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};
use uuid::Uuid;

use super::handshake::DeviceInfo;
use super::AppState;

const PLACENET_INIT_HEADER: &str = "x-placenet-init";
const SUPPORTED_VERSION: &str = "0.0.1";

#[derive(Serialize)]
struct InitResponse {
    cert_pem: String,
    brokerage: super::handshake::MqttBrokerageInfo,
}

#[derive(Deserialize)]
pub(super) struct ClientRegisterRequest {
    csr_pem: String,
}

#[derive(Serialize)]
struct ClientRegisterResponse {
    client_id: String,
    cert_pem: String,
    ca_cert_pem: String,
}

async fn parse_device_info(body: Body) -> Result<DeviceInfo, Response> {
    let bytes = axum::body::to_bytes(body, 64 * 1024)
        .await
        .map_err(|e| {
            error!("Failed to read request body: {}", e);
            (StatusCode::BAD_REQUEST, "Failed to read request body".to_string()).into_response()
        })?;

    serde_json::from_slice(&bytes).map_err(|e| {
        warn!("Invalid device registration payload: {}", e);
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Invalid JSON body: {}", e),
        )
            .into_response()
    })
}

/// `GET /health`
///
/// Simple liveness probe to confirm the HTTP server is reachable.
pub async fn health() -> impl IntoResponse {
    (StatusCode::OK, "Hello, World!")
}

/// `POST /`
///
/// Expects the `X-PlaceNet-Init: <version>` header and a JSON body matching [`DeviceInfo`].
/// Signs the device's CSR, stores the cert, and returns it alongside MQTT brokerage info.
pub async fn init_device(
    State(state): State<AppState>,
    request: Request,
) -> impl IntoResponse {
    // ── 1. Validate the protocol-version header ──────────────────────────
    let version = match request.headers().get(PLACENET_INIT_HEADER) {
        Some(v) => match v.to_str() {
            Ok(s) => s.to_owned(),
            Err(_) => {
                warn!("X-PlaceNet-Init header contains non-ASCII bytes");
                return (
                    StatusCode::BAD_REQUEST,
                    "X-PlaceNet-Init header must be valid ASCII",
                )
                    .into_response();
            }
        },
        None => {
            warn!("Request missing X-PlaceNet-Init header");
            return (
                StatusCode::BAD_REQUEST,
                "Missing required header: X-PlaceNet-Init",
            )
                .into_response();
        }
    };

    if version != SUPPORTED_VERSION {
        warn!("Unsupported PlaceNet init version: {}", version);
        return (
            StatusCode::BAD_REQUEST,
            format!("Unsupported version '{}'; expected '{}'", version, SUPPORTED_VERSION),
        )
            .into_response();
    }

    // ── 2. Parse JSON body ───────────────────────────────────────────────
    let device = match parse_device_info(request.into_body()).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };

    let device_id = &device.mdns.hostname;

    info!(
        address = %device.address,
        mdns_hostname = %device_id,
        mdns_port = device.mdns.port,
        "Device handshake — signing CSR"
    );

    // ── 3. Sign the CSR and store the cert ──────────────────────────────
    let cert_pem = match state.ca.sign_csr(device_id, &device.csr_pem).await {
        Ok(pem) => pem,
        Err(e) => {
            error!(device_id, "Failed to sign CSR: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to sign device certificate")
                .into_response();
        }
    };

    // ── 4. Return signed cert + MQTT brokerage information ───────────────
    (
        StatusCode::OK,
        Json(InitResponse {
            cert_pem,
            brokerage: state.brokerage_info,
        }),
    )
        .into_response()
}

/// `POST /peer/message`
///
/// Receives a plain-text message from a peer placenet-home node and logs it.
pub async fn peer_message(body: axum::body::Bytes) -> impl IntoResponse {
    let text = String::from_utf8_lossy(&body);
    info!(message = %text, "Received peer message");
    (StatusCode::OK, "Message received")
}

/// `POST /.well-known/placenet/register`
///
/// Client registration endpoint. Accepts a JSON body containing a PEM-encoded CSR,
/// issues a CA-signed certificate, and returns the client's assigned ID, its cert,
/// and the CA cert for trust anchoring.
pub(super) async fn register_client(
    State(state): State<AppState>,
    Json(payload): Json<ClientRegisterRequest>,
) -> impl IntoResponse {
    let client_id = Uuid::new_v4().to_string();

    let cert_pem = match state.ca.sign_csr(&client_id, &payload.csr_pem).await {
        Ok(pem) => pem,
        Err(e) => {
            error!(client_id, "Failed to sign client CSR: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to sign client certificate")
                .into_response();
        }
    };

    let ca_cert_pem = match state.ca.ca_cert_pem().await {
        Ok(pem) => pem,
        Err(e) => {
            error!("Failed to retrieve CA cert: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to retrieve CA certificate")
                .into_response();
        }
    };

    info!(client_id, "Client registered successfully");

    (
        StatusCode::CREATED,
        Json(ClientRegisterResponse { client_id, cert_pem, ca_cert_pem }),
    )
        .into_response()
}
