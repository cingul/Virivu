use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

use crate::auth::AuthError;

#[derive(Debug)]
pub enum ApiError {
    Auth(AuthError),
    Validation(String),
    NotFound(String),
    Conflict(String),
    Internal(String),
}

impl ApiError {
    pub fn internal<E: ToString>(error: E) -> Self {
        Self::Internal(error.to_string())
    }
}

pub fn map_db_error(error: anyhow::Error) -> ApiError {
    let message = error.to_string();
    if let Some(conflict) = message.strip_prefix("conflict:") {
        return ApiError::Conflict(conflict.trim().to_string());
    }
    if let Some(validation) = message.strip_prefix("validation:") {
        return ApiError::Validation(validation.trim().to_string());
    }
    ApiError::Internal(message)
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            Self::Auth(error) => error.into_response(),
            Self::Validation(msg) => (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": msg })),
            )
                .into_response(),
            Self::NotFound(msg) => (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": msg })),
            )
                .into_response(),
            Self::Conflict(msg) => (
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "error": msg })),
            )
                .into_response(),
            Self::Internal(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": msg })),
            )
                .into_response(),
        }
    }
}
