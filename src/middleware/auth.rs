use axum::{
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
    extract::Request,
    body::Body,
};
use tower_sessions::Session;
use tracing;

pub async fn require_auth(
    session: Session,
    req: Request<Body>,
    next: Next,
) -> Response {
    let path = req.uri().path();

    if path == "/" || path == "/login" || path == "/register" {
        return next.run(req).await;
    }
    
    match session.get::<String>("user_session").await {
        Ok(Some(username)) => {
            tracing::debug!("Authenticated request from user '{}' to path: {}", username, path);
            next.run(req).await
        }
        Ok(None) => {
            tracing::warn!("Unauthenticated request to protected path: {}", path);
            Redirect::to("/?error=Please%20login%20to%20continue").into_response()
        }
        Err(e) => {
            tracing::error!("Session error in auth middleware: {}", e);
            Redirect::to("/?error=Session%20error%2C%20please%20login%20again").into_response()
        }
    }
}
