use mongodb::Database;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct AppState {
    pub db: Database,
    pub jwt_secret: String,
    pub model_fast: Arc<Mutex<ort::session::Session>>,
    pub model_refined: Arc<Mutex<ort::session::Session>>,
}
