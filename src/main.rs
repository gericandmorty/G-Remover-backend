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

    // ── Load RMBG-1.4 (quantized, 1024×1024) ────────────────────────────────
    // Single-model pipeline: only the 42 MB quantized model runs.
    // This keeps peak RAM well under 512 MB on Render's free tier.
    let model_path = "assets/rmbg-1.4.onnx";
    if !std::path::Path::new(model_path).exists() {
        tracing::error!(
            "Model not found at: {}. Download it with:\n  \
            wget -O assets/rmbg-1.4.onnx \"https://huggingface.co/briaai/RMBG-1.4/resolve/main/onnx/model_quantized.onnx\"",
            model_path
        );
        std::process::exit(1);
    }
    tracing::info!("Loading RMBG-1.4 (quantized) model...");
    let model_session = {
        let builder = ort::session::Session::builder().unwrap_or_else(|e| {
            tracing::error!("Failed to create ONNX session builder: {}", e);
            std::process::exit(1);
        });
        let builder = builder
            .with_execution_providers([
                ort::execution_providers::CPUExecutionProvider::default()
                    .with_arena_allocator(false)
                    .build()
            ])
            .unwrap_or_else(|e| {
                tracing::error!("Failed to set execution providers: {}", e);
                std::process::exit(1);
            });
        let builder = builder
            .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Disable)
            .unwrap_or_else(|e| {
                tracing::error!("Failed to set optimization level: {}", e);
                std::process::exit(1);
            });
        let builder = builder.with_memory_pattern(false).unwrap_or_else(|e| {
            tracing::error!("Failed to disable memory pattern: {}", e);
            std::process::exit(1);
        });
        let builder = builder
            .with_config_entry("session.use_memory_arena", "0")
            .unwrap_or_else(|e| {
                tracing::error!("Failed to disable memory arena: {}", e);
                std::process::exit(1);
            });
        let builder = builder
            .with_config_entry("session.use_arena_allocation", "0")
            .unwrap_or_else(|e| {
                tracing::error!("Failed to disable arena allocation: {}", e);
                std::process::exit(1);
            });
        let mut builder = builder.with_intra_threads(1).unwrap_or_else(|e| {
            tracing::error!("Failed to set intra threads: {}", e);
            std::process::exit(1);
        });
        match builder.commit_from_file(model_path) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Failed to load RMBG-1.4 model: {}", e);
                std::process::exit(1);
            }
        }
    };
    let model = std::sync::Arc::new(tokio::sync::Mutex::new(model_session));
    tracing::info!("RMBG-1.4 (quantized) loaded successfully.");

    // Initialize shared AppState
    let state = state::AppState {
        db,
        jwt_secret: config.jwt_secret.clone(),
        model,
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
