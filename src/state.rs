use mongodb::Database;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct AppState {
    pub db: Database,
    pub jwt_secret: String,
    /// Phase 1 — u2netp (320×320, fast rough cut)
    /// Session::run requires &mut self, so we wrap it in a Mutex.
    pub model_fast: Arc<Mutex<ort::session::Session>>,
    /// Phase 2 — BRIA RMBG-1.4 (1024×1024, refined edge cleanup)
    pub model_refined: Arc<Mutex<ort::session::Session>>,
}
