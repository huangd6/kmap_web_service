mod models;
mod handlers;
mod services;
mod middleware;
mod worker;
mod kmap_algorithms;
mod config;
mod errors;

use axum::{
    routing::{get, post},
    Router,
    extract::DefaultBodyLimit,
    middleware::from_fn,
};
use tower_http::{
    services::ServeDir,
    limit::RequestBodyLimitLayer,
};
use tower_sessions::{MemoryStore, SessionManagerLayer};
use tower_sessions::cookie::SameSite;
use std::sync::Arc;
use tokio::sync::Semaphore;
use crate::{
    services::RedisService,
    config::Config,
};
use tracing_subscriber;

#[tokio::main]
async fn main() {
    // Initialize basic tracing subscriber
    tracing_subscriber::fmt::init();

    // Load configuration
    let config = Config::load().expect("Failed to load configuration");
    let config_state = config.clone();

    // Initialize Redis client
    let redis_client = if config.redis.sentinel_enabled {
        Arc::new(redis::Client::open(
            config.redis.sentinel_url.expect("Sentinel URL not configured")
        ).expect("Failed to connect to Redis Sentinel"))
    } else {
        Arc::new(redis::Client::open(config.redis.url)
            .expect("Failed to connect to Redis"))
    };
    
    // Initialize RedisService
    let redis_service = RedisService::new(redis_client.clone());
    
    // Initialize worker pool with configured values
    let semaphore = Arc::new(Semaphore::new(config.worker.max_concurrent_tasks));

    // Initialize worker pool
    for _ in 0..config.worker.worker_count {
        let redis_service_worker = redis_service.clone();
        let semaphore_worker = semaphore.clone();
        tokio::spawn(async move {
            worker::worker_process(redis_service_worker, semaphore_worker).await;
        });
    }
    
    // Session store setup
    let session_store = MemoryStore::default();
    let session_layer = SessionManagerLayer::new(session_store)
        .with_secure(false)
        .with_same_site(SameSite::Lax)
        .with_name("session");

    // Create router with all routes
    let app = Router::new()
        // Auth routes
        .route("/", get(handlers::serve_login_page))
        .route("/login", post(handlers::handle_login))
        .route("/register", post(handlers::handle_register))
        .route("/logout", get(handlers::handle_logout))
        
        // Task routes
        .route("/upload", get(handlers::serve_upload_page))
        .route("/process", post(handlers::process_upload))
        .route("/status/:task_id", get(handlers::get_task_status))
        .route("/download/:task_id", get(handlers::download_results))
        
        // Dashboard routes
        .route("/process/:task_id", get(handlers::view_process))
        .route("/delete/:task_id", get(handlers::delete_task))
        .route("/user", get(handlers::serve_user_dashboard))
        
        // Static files
        .nest_service("/static", ServeDir::new("static"))
        
        // Add middleware
        .layer(from_fn(middleware::require_auth))
        .layer(session_layer)
        
        // File upload limits from config
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(config.upload.max_file_size))
        
        // Add state
        .with_state((redis_service, config_state));

    println!("Server running");
    let listener = tokio::net::TcpListener::bind(
        format!("{}:{}", config.server.host, config.server.port)
    )
    .await
    .expect("Failed to bind server");

    axum::serve(listener, app.into_make_service())
        .await
        .expect("Failed to start server");
}

// Application state that can be shared between handlers
//#[derive(Clone)]
//struct AppState {
//    redis_service: Arc<services::RedisService>,
//    task_service: Arc<services::TaskService>,
//} 