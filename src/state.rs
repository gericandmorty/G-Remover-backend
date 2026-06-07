use mongodb::Database;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct AppState {
    pub db: Database,
    pub jwt_secret: String,
    // Session::run requires &mut self, so we wrap it in a Mutex
    pub model: Arc<Mutex<ort::session::Session>>,
}
