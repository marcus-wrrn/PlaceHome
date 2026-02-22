use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;

use crate::models::ApiResponse;
use crate::AppState;
use crate::supervisor::ServiceStatus;

use std::collections::HashMap;
use crate::services::ServiceId;

/// Detailed health check showing per-service status
pub async fn health(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<HealthInfo>>, StatusCode> {
    let services = state.supervisor.get_status().await.map_err(|_| {
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let mqtt_connected = state.mqtt.is_some();

    let info = HealthInfo {
        services,
        mqtt_connected,
    };

    Ok(Json(ApiResponse {
        success: true,
        data: info,
    }))
}

#[derive(serde::Serialize)]
pub struct HealthInfo {
    pub services: HashMap<ServiceId, ServiceStatus>,
    pub mqtt_connected: bool,
}
