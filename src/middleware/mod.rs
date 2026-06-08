use axum::http::{header, Method};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

pub mod rate_limit;
pub use rate_limit::RateLimitLayer;

// Configure CORS policy for frontend connection
pub fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
}

// Configure HTTP request/response logging middleware
pub fn trace_layer() -> TraceLayer<
    tower_http::classify::SharedClassifier<tower_http::classify::ServerErrorsAsFailures>,
> {
    TraceLayer::new_for_http()
}
