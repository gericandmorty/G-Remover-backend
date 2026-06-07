mod config;
mod errors;
mod middleware;
mod models;
mod routes;
mod state;

use config::Config;
use std::net::SocketAddr;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() {
    // Load environment variables from .env file
    if let Err(e) = dotenvy::dotenv() {
        println!("Warning: failed to load .env file: {}", e);
    }

    // Initialize structured logging and tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "backend=info,tower_http=info,axum=info,mongodb=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load configuration
    let config = Config::from_env();
    let addr = SocketAddr::from((config.host, config.port));

    // Connect to MongoDB
    tracing::info!("Connecting to MongoDB database...");
    let client_options = match mongodb::options::ClientOptions::parse(&config.mongodb_uri).await {
        Ok(opt) => opt,
        Err(e) => {
            tracing::error!("Failed to parse MongoDB connection URI: {}", e);
            std::process::exit(1);
        }
    };
    
    let client = match mongodb::Client::with_options(client_options) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to initialize MongoDB client: {}", e);
            std::process::exit(1);
        }
    };
    
    let db = client.database(&config.mongodb_db_name);
    tracing::info!("Successfully connected to database: {}", config.mongodb_db_name);

    // Initialize shared AppState
    let state = state::AppState {
        db,
        jwt_secret: config.jwt_secret.clone(),
    };

    // Construct application router, bind state, and apply middleware
    let app = routes::app_router()
        .with_state(state)
        .layer(middleware::cors_layer())
        .layer(middleware::trace_layer());

    // Bind TCP listener
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("Failed to bind to {}: {}", addr, e);
            std::process::exit(1);
        }
    };

    tracing::info!("Server successfully initialized and running on http://{}", addr);

    // Run the server with graceful shutdown handling
    if let Err(err) = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
    {
        tracing::error!("Server error: {}", err);
    }
}

// Handler for listening to termination signals (graceful shutdown)
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("Ctrl+C signal received, shutting down gracefully...");
        },
        _ = terminate => {
            tracing::info!("Termination signal received, shutting down gracefully...");
        },
    }
}
