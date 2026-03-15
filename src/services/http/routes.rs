use axum::{
    Json,
    body::Body,
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use tracing::{error, info, warn};

use super::handshake::DeviceInfo;
use super::AppState;

const PLACENET_INIT_HEADER: &str = "x-placenet-init";
const SUPPORTED_VERSION: &str = "0.0.1";

#[derive(Serialize)]
struct InitResponse {
    cert_pem: String,
    brokerage: super::handshake::MqttBrokerageInfo,
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
