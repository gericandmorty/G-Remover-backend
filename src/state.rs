use mongodb::Database;

#[derive(Clone)]
pub struct AppState {
    pub db: Database,
    pub jwt_secret: String,
}
