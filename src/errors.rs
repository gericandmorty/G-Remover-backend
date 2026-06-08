use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

#[derive(Error, Debug)]
#[allow(dead_code)]
pub enum AppError {
    /// 500 — unexpected internal failures; message is logged but NOT sent to client
    #[error("Internal Server Error: {0}")]
    Internal(String),

    /// 404
    #[error("Not Found: {0}")]
    NotFound(String),

    /// 400 — caller made a bad request (message IS sent to client)
    #[error("Bad Request: {0}")]
    BadRequest(String),

    /// 415 — file type not accepted
    #[error("Unsupported Media Type: {0}")]
    UnsupportedMediaType(String),

    /// 413 — payload too large
    #[error("Payload Too Large: {0}")]
    PayloadTooLarge(String),

    /// 422 — semantically invalid request
    #[error("Unprocessable Entity: {0}")]
    UnprocessableEntity(String),

    /// 500 — MongoDB driver errors; details logged, generic message sent
    #[error("Database error: {0}")]
    Database(#[from] mongodb::error::Error),

    /// 401
    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    /// 409
    #[error("Conflict: {0}")]
    Conflict(String),

    /// 429 — rate limit exceeded
    #[error("Too Many Requests: {0}")]
    RateLimited(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, client_message) = match self {
            AppError::Internal(ref msg) => {
                tracing::error!("Internal error: {}", msg);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Something went wrong on our end. Please try again.".to_string(),
                )
            }
            AppError::Database(ref err) => {
                tracing::error!("Database error: {}", err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "A database error occurred. Please try again.".to_string(),
                )
            }
            AppError::NotFound(ref msg) => (StatusCode::NOT_FOUND, msg.clone()),
            AppError::BadRequest(ref msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::UnsupportedMediaType(ref msg) => (StatusCode::UNSUPPORTED_MEDIA_TYPE, msg.clone()),
            AppError::PayloadTooLarge(ref msg) => (StatusCode::PAYLOAD_TOO_LARGE, msg.clone()),
            AppError::UnprocessableEntity(ref msg) => (StatusCode::UNPROCESSABLE_ENTITY, msg.clone()),
            AppError::Unauthorized(ref msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
            AppError::Conflict(ref msg) => (StatusCode::CONFLICT, msg.clone()),
            AppError::RateLimited(ref msg) => (StatusCode::TOO_MANY_REQUESTS, msg.clone()),
        };

        let body = Json(json!({
            "status": "error",
            "message": client_message,
        }));

        (status, body).into_response()
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
