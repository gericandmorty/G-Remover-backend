use mongodb::Database;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct AppState {
    pub db: Database,
    pub jwt_secret: String,
    /// RMBG-1.4 8-bit quantized model (~42 MB) — single-phase pipeline
    pub model: Arc<Mutex<ort::session::Session>>,
}
