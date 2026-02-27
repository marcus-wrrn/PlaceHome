use axum::{
    body::Body,
    extract::Request,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use tracing::{error, info, warn};

use super::handshake::{DeviceInfo, initiate_tls_handshake};

const PLACENET_INIT_HEADER: &str = "x-placenet-init";
const SUPPORTED_VERSION: &str = "0.0.1";

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
/// Simple liveness probe to confirm the HTTPS server is reachable.
pub async fn health() -> impl IntoResponse {
    (StatusCode::OK, "Hello, World!")
}

/// `POST /`
///
/// Expects the `X-PlaceNet-Init: <version>` header and a JSON body
/// matching [`DeviceInfo`].  On success the server initiates a TLS
/// handshake back to the device.
pub async fn init_device(request: Request) -> impl IntoResponse {
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

    info!(
        address = %device.address,
        mdns_hostname = %device.mdns.hostname,
        "Device registration received, initiating TLS handshake"
    );

    // ── 3. Initiate TLS handshake ────────────────────────────────────────
    match initiate_tls_handshake(&device).await {
        Ok(_tls_stream) => {
            info!(address = %device.address, "Secure channel established");
            (StatusCode::OK, "Secure channel established").into_response()
        }
        Err(e) => {
            error!("Failed to establish secure channel with {}: {}", device.address, e);
            (
                StatusCode::BAD_GATEWAY,
                format!("Could not establish secure channel: {}", e),
            )
                .into_response()
        }
    }
}
