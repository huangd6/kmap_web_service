use axum::{
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
    extract::Request,
    body::Body,
};
use tower_sessions::Session;

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
        Ok(Some(_)) => next.run(req).await,
        _ => Redirect::to("/").into_response(),
    }
}
