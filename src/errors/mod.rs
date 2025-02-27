// Defines a custom error type and a result type alias for a Rust application using the thiserror crate.
use thiserror::Error;

// Make the response module public
pub mod response;
pub mod worker;

// Re-export commonly used types
pub use worker::{WorkerError, WorkerResult};

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Authentication error: {0}")]
    Auth(String),

    // The #[from] attribute automatically converts a redis::RedisError into an AppError::Redis using the From trait.
    #[error("Redis error: {0}")]
    Redis(#[from] redis::RedisError),

    #[error("File error: {0}")]
    File(#[from] std::io::Error),

    #[error("Task error: {0}")]
    Task(String),

    #[error("Upload error: {0}")]
    Upload(String),

    #[error("Worker error: {0}")]
    Worker(#[from] WorkerError),
}

// Custom result type
pub type AppResult<T> = Result<T, AppError>;