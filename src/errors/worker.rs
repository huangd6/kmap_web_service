use thiserror::Error;
use redis::RedisError;
use std::io;

#[derive(Error, Debug)]
pub enum WorkerError {
    #[error("Task timed out after {0} seconds")]
    Timeout(u64),

    #[error("Task panicked: {0}")]
    TaskPanic(String),

    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("Processing error: {0}")]
    Processing(String),

    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Redis error: {0}")]
    Redis(#[from] RedisError),

    #[error("Invalid k-mer: {0}")]
    InvalidKmer(String),

    #[error("Quota exceeded: user {0} has no remaining quota")]
    QuotaExceeded(String),
}

pub type WorkerResult<T> = Result<T, WorkerError>;
