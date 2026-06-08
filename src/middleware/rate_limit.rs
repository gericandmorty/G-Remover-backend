use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, Response, StatusCode},
    response::IntoResponse,
};
use dashmap::DashMap;
use futures::future::BoxFuture;
use std::{
    net::SocketAddr,
    sync::Arc,
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tower::{Layer, Service};

/// Per-IP bucket state
struct Bucket {
    tokens: f64,
    last_refill: Instant,
}

/// Shared state across all clones of the middleware
#[derive(Clone)]
struct RateLimitState {
    buckets: Arc<DashMap<String, Bucket>>,
    /// Maximum tokens (= burst capacity)
    capacity: f64,
    /// Tokens added per second (= sustained rate)
    refill_rate: f64,
}

impl RateLimitState {
    fn new(max_requests_per_minute: u32) -> Self {
        // Allow short bursts up to 2× the per-minute rate
        let capacity = (max_requests_per_minute * 2) as f64;
        let refill_rate = max_requests_per_minute as f64 / 60.0;
        Self {
            buckets: Arc::new(DashMap::new()),
            capacity,
            refill_rate,
        }
    }

    /// Returns `true` if the request from this IP is allowed.
    fn check_and_consume(&self, ip: &str) -> bool {
        let now = Instant::now();

        let mut entry = self.buckets.entry(ip.to_string()).or_insert_with(|| Bucket {
            tokens: self.capacity,
            last_refill: now,
        });

        // Refill tokens based on elapsed time
        let elapsed = now.duration_since(entry.last_refill).as_secs_f64();
        entry.tokens = (entry.tokens + elapsed * self.refill_rate).min(self.capacity);
        entry.last_refill = now;

        if entry.tokens >= 1.0 {
            entry.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Prune entries that haven't been touched in the last 10 minutes to avoid
    /// unbounded memory growth.
    fn prune_stale(&self) {
        let cutoff = Duration::from_secs(600);
        self.buckets
            .retain(|_, v| v.last_refill.elapsed() < cutoff);
    }
}

// ─── Layer ───────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct RateLimitLayer {
    state: RateLimitState,
}

impl RateLimitLayer {
    /// `max_requests_per_minute` — how many requests a single IP may make per
    /// minute on a sustained basis. Burst allows up to 2× that in a short window.
    pub fn new(max_requests_per_minute: u32) -> Self {
        let state = RateLimitState::new(max_requests_per_minute);

        // Background task: prune stale entries every 5 minutes
        let state_clone = state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(300));
            loop {
                interval.tick().await;
                state_clone.prune_stale();
            }
        });

        Self { state }
    }
}

impl<S> Layer<S> for RateLimitLayer {
    type Service = RateLimitMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RateLimitMiddleware {
            inner,
            state: self.state.clone(),
        }
    }
}

// ─── Service ─────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct RateLimitMiddleware<S> {
    inner: S,
    state: RateLimitState,
}

impl<S> Service<Request<Body>> for RateLimitMiddleware<S>
where
    S: Service<Request<Body>, Response = Response<Body>> + Send + Clone + 'static,
    S::Future: Send + 'static,
{
    type Response = Response<Body>;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        // Extract client IP from ConnectInfo extension (set by axum::serve)
        // Fall back to a placeholder so the middleware never panics
        let ip = req
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ci| ci.0.ip().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let allowed = self.state.check_and_consume(&ip);

        if !allowed {
            let response = (
                StatusCode::TOO_MANY_REQUESTS,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                r#"{"status":"error","message":"Rate limit exceeded. Please slow down and try again shortly."}"#,
            )
                .into_response();

            return Box::pin(async move { Ok(response) });
        }

        let future = self.inner.call(req);
        Box::pin(async move { future.await })
    }
}
