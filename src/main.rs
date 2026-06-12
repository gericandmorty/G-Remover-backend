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

    // ── Load Phase 1 model: u2netp (fast rough cut, 320×320) ─────────────────
    tracing::info!("Loading Phase 1 model (u2netp) from assets/u2netp.onnx...");
    let fast_model_path = "assets/u2netp.onnx";
    if !std::path::Path::new(fast_model_path).exists() {
        tracing::error!(
            "Phase 1 model not found at: {}. \
             Download it from the U2-Net repository or project assets.",
            fast_model_path
        );
        std::process::exit(1);
    }
    let model_fast_session = {
        let builder = ort::session::Session::builder().unwrap_or_else(|e| {
            tracing::error!("Failed to create ONNX session builder (fast): {}", e);
            std::process::exit(1);
        });
        let builder = builder
            .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3)
            .unwrap_or_else(|e| {
                tracing::error!("Failed to set optimization level (fast): {}", e);
                std::process::exit(1);
            });
        let mut builder = builder.with_intra_threads(2).unwrap_or_else(|e| {
            tracing::error!("Failed to set intra threads (fast): {}", e);
            std::process::exit(1);
        });
        match builder.commit_from_file(fast_model_path) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Failed to load u2netp model: {}", e);
                std::process::exit(1);
            }
        }
    };
    let model_fast = std::sync::Arc::new(tokio::sync::Mutex::new(model_fast_session));
    tracing::info!("Phase 1 model (u2netp) loaded successfully.");

    // ── Load Phase 2 model: BRIA RMBG-1.4 (refined cleanup, 1024×1024) ───────
    tracing::info!("Loading Phase 2 model (RMBG-1.4) from assets/rmbg-1.4.onnx...");
    let refined_model_path = "assets/rmbg-1.4.onnx";
    if !std::path::Path::new(refined_model_path).exists() {
        tracing::error!(
            "Phase 2 model not found at: {}. Download it with:\n  \
            wget -O assets/rmbg-1.4.onnx \"https://huggingface.co/briaai/RMBG-1.4/resolve/main/onnx/model.onnx\"",
            refined_model_path
        );
        std::process::exit(1);
    }
    let model_refined_session = {
        let builder = ort::session::Session::builder().unwrap_or_else(|e| {
            tracing::error!("Failed to create ONNX session builder (refined): {}", e);
            std::process::exit(1);
        });
        let builder = builder
            .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3)
            .unwrap_or_else(|e| {
                tracing::error!("Failed to set optimization level (refined): {}", e);
                std::process::exit(1);
            });
        let mut builder = builder.with_intra_threads(2).unwrap_or_else(|e| {
            tracing::error!("Failed to set intra threads (refined): {}", e);
            std::process::exit(1);
        });
        match builder.commit_from_file(refined_model_path) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Failed to load RMBG-1.4 model: {}", e);
                std::process::exit(1);
            }
        }
    };
    let model_refined = std::sync::Arc::new(tokio::sync::Mutex::new(model_refined_session));
    tracing::info!("Phase 2 model (RMBG-1.4) loaded successfully.");

    // Initialize shared AppState
    let state = state::AppState {
        db,
        jwt_secret: config.jwt_secret.clone(),
        model_fast,
        model_refined,
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
    if let Err(err) = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
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
