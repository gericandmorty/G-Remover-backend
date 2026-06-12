pub mod auth;
pub mod remove;

use axum::{
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde_json::json;
use std::time::SystemTime;

use crate::errors::{AppError, Result};
use crate::middleware::RateLimitLayer;
use crate::state::AppState;

pub fn app_router() -> Router<AppState> {
    // Apply rate limiting only to the unauthenticated remove-background endpoint.
    // 10 requests/minute per IP on a sustained basis; burst up to 20.
    let remove_bg_route = Router::new()
        .route("/api/v1/remove-background", post(remove::remove_handler))
        .layer(RateLimitLayer::new(10));

    Router::new()
        .route("/api/health", get(health_handler))
        .route("/api/info", get(info_handler))
        .route("/api/error-demo", get(error_demo_handler))
        .route("/api/auth/register", post(auth::register_handler))
        .route("/api/auth/login", post(auth::login_handler))
        .merge(remove_bg_route)
}

// Serves system health status
async fn health_handler() -> impl IntoResponse {
    Json(json!({
        "status": "ok",
        "timestamp": SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        "service": "g-remover-backend"
    }))
}

// Serves system info
async fn info_handler() -> impl IntoResponse {
    Json(json!({
        "app_name": "G-Remover API",
        "version": env!("CARGO_PKG_VERSION"),
        "framework": "Axum 0.7",
        "runtime": "Tokio",
        "status": "operational",
        "endpoints": [
            { "method": "GET", "path": "/api/health", "description": "Liveness probe" },
            { "method": "GET", "path": "/api/info", "description": "API details and version" },
            { "method": "GET", "path": "/api/error-demo", "description": "Demonstrates error response mapping" },
            { "method": "POST", "path": "/api/auth/register", "description": "User registration" },
            { "method": "POST", "path": "/api/auth/login", "description": "User authentication and JWT generation" },
            { "method": "POST", "path": "/api/v1/remove-background", "description": "Authenticates and removes image backgrounds" }
        ]
    }))
}

// Demonstrates the error handling system
async fn error_demo_handler() -> Result<&'static str> {
    Err(AppError::BadRequest("This is a mock bad request error to showcase error response formatting".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    // Helper to generate a dummy AppState for routing test compatibility.
    async fn get_mock_state() -> AppState {
        let client = mongodb::Client::with_uri_str("mongodb://localhost:27017")
            .await
            .unwrap();
        let db = client.database("test_db");

        let session = ort::session::Session::builder()
            .unwrap()
            .with_execution_providers([
                ort::execution_providers::CPUExecutionProvider::default()
                    .with_arena_allocator(false)
                    .build()
            ])
            .unwrap()
            .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Disable)
            .unwrap()
            .with_memory_pattern(false)
            .unwrap()
            .with_config_entry("session.use_memory_arena", "0")
            .unwrap()
            .with_config_entry("session.use_arena_allocation", "0")
            .unwrap()
            .with_intra_threads(1)
            .unwrap()
            .commit_from_file("assets/u2netp.onnx") // stub — any small model works for routing tests
            .unwrap();
        let model = std::sync::Arc::new(tokio::sync::Mutex::new(session));

        AppState {
            db,
            jwt_secret: "mock_jwt_secret_key_for_testing".to_string(),
            model,
        }
    }

    #[tokio::test]
    async fn test_health_check() {
        let state = get_mock_state().await;
        let app = app_router().with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), 2048)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
        assert_eq!(json["service"], "g-remover-backend");
    }

    #[tokio::test]
    async fn test_info() {
        let state = get_mock_state().await;
        let app = app_router().with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/info")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), 2048)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["app_name"], "G-Remover API");
        assert_eq!(json["status"], "operational");
    }

    #[tokio::test]
    async fn test_error_demo() {
        let state = get_mock_state().await;
        let app = app_router().with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/error-demo")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = axum::body::to_bytes(response.into_body(), 2048)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "error");
        assert_eq!(json["message"], "This is a mock bad request error to showcase error response formatting");
    }
}
