use std::env;
use std::net::IpAddr;

#[derive(Clone, Debug)]
pub struct Config {
    pub host: IpAddr,
    pub port: u16,
    pub mongodb_uri: String,
    pub mongodb_db_name: String,
    pub jwt_secret: String,
}

impl Config {
    pub fn from_env() -> Self {
        let host = env::var("HOST")
            .unwrap_or_else(|_| "0.0.0.0".to_string())
            .parse::<IpAddr>()
            .expect("HOST must be a valid IP address");

        let port = env::var("PORT")
            .unwrap_or_else(|_| "8080".to_string())
            .parse::<u16>()
            .expect("PORT must be a valid 16-bit unsigned integer");

        let mongodb_uri = env::var("MONGODB_URI")
            .expect("MONGODB_URI environment variable is required");

        let mongodb_db_name = env::var("MONGODB_DB_NAME")
            .unwrap_or_else(|_| "g_remover".to_string());

        let jwt_secret = env::var("JWT_SECRET")
            .expect("JWT_SECRET environment variable is required");

        Config {
            host,
            port,
            mongodb_uri,
            mongodb_db_name,
            jwt_secret,
        }
    }
}
