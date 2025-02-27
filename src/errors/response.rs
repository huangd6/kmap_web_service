use axum::{
    response::{IntoResponse, Response, Redirect},
    http::StatusCode,
};
use urlencoding;
use crate::errors::{
    AppError,
    worker::WorkerError,
};

// The IntoResponse trait implementation converts AppError into a well-formed HTTP response.
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            // Authentication errors redirect to login
            AppError::Auth(msg) => {
                Redirect::to(&format!("/?error={}", urlencoding::encode(&msg)))
                    .into_response()
            }

            // Database errors are internal server errors
            AppError::Redis(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database error: {}", e)
            ).into_response(),

            // File and upload errors are bad requests
            AppError::File(e) => (
                StatusCode::BAD_REQUEST,
                format!("File error: {}", e)
            ).into_response(),

            AppError::Upload(msg) => (
                StatusCode::BAD_REQUEST,
                format!("Upload error: {}", msg)
            ).into_response(),

            // Task errors are bad requests
            AppError::Task(msg) => (
                StatusCode::BAD_REQUEST,
                format!("Task error: {}", msg)
            ).into_response(),

            // Worker errors have specific status codes
            AppError::Worker(err) => convert_worker_error(err),
        }
    }
}

// Helper function to convert worker errors to responses
fn convert_worker_error(err: WorkerError) -> Response {
    match err {
        WorkerError::Timeout(seconds) => (
            StatusCode::REQUEST_TIMEOUT,
            format!("Task processing timed out after {} seconds", seconds)
        ).into_response(),

        WorkerError::QuotaExceeded(user) => (
            StatusCode::FORBIDDEN,
            format!("Processing quota exceeded for user {}", user)
        ).into_response(),

        WorkerError::FileNotFound(path) => (
            StatusCode::NOT_FOUND,
            format!("File not found: {}", path)
        ).into_response(),

        WorkerError::InvalidKmer(msg) => (
            StatusCode::BAD_REQUEST,
            format!("Invalid k-mer: {}", msg)
        ).into_response(),

        // All other worker errors are internal server errors
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Worker error: {}", err)
        ).into_response(),
    }
}