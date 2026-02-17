use axum::{http::StatusCode, Json};
use crate::models::{ApiResponse, ErrorResponse};

pub async fn health() -> Result<Json<ApiResponse<String>>, (StatusCode, Json<ErrorResponse>)> {
    Ok(Json(ApiResponse {
        success: true,
        data: "ok".to_string(),
    }))
}
