use axum::{
    extract::{Form, State},
    response::{Html, IntoResponse, Response, Redirect},
};
use tower_sessions::Session;
use std::fs;
use bcrypt::{hash, verify, DEFAULT_COST};
use crate::models::{LoginForm, RegisterForm, User};
use crate::services::RedisService;
use crate::errors::{AppError, AppResult};
use crate::config::Config;

pub async fn serve_login_page() -> AppResult<Response> {
    let login_html = fs::read_to_string("templates/login.html")
        .map_err(|e| AppError::File(e))?;
    Ok(Html(login_html).into_response())
}

#[axum::debug_handler]
pub async fn handle_login(
    State((redis_service, _)): State<(RedisService, Config)>,
    session: Session,
    Form(login_form): Form<LoginForm>,
) -> AppResult<Response> {
    tracing::info!("Login attempt for user: {}", login_form.username);
    
    // Get user from Redis
    let user = redis_service
        .get_user(&login_form.username)
        .await?  // Redis errors are automatically converted to AppError::Redis
        .ok_or_else(|| AppError::Auth("Invalid username".into()))?;
    
    // Verify password
    if !verify(&login_form.password, &user.password_hash).unwrap_or(false) {
        tracing::warn!("Invalid password for user: {}", login_form.username);
        return Err(AppError::Auth("Invalid username or password".into()));
    }

    // Set session
    session
        .insert("user_session", login_form.username)
        .await
        .map_err(|e| AppError::Auth(format!("Session error: {}", e)))?;

    // Successful login
    tracing::info!("Successful login for user: {}", user.username);
    Ok(Redirect::to("/user").into_response())
}

pub async fn handle_register(
    State((redis_service, _)): State<(RedisService, Config)>,
    Form(register_form): Form<RegisterForm>,
) -> AppResult<Response> {
    tracing::info!("Registration attempt for user: {}", register_form.username);

    // Check if user exists first
    if let Some(_) = redis_service.get_user(&register_form.username).await? {
        tracing::warn!("Username already taken: {}", register_form.username);
        return Err(AppError::Auth("Username already taken".into()));
    }
    
    // Then validate passwords match
    if register_form.password != register_form.confirm_password {
        tracing::warn!("Password mismatch during registration for user: {}", register_form.username);
        return Err(AppError::Auth("Passwords don't match".into()));
    }
    
    // Create new user with password hash
    let password_hash = hash(register_form.password.as_bytes(), DEFAULT_COST)
        .map_err(|e| {
            tracing::error!("Password hashing failed: {}", e);
            AppError::Auth("Registration failed: password processing error".into())
        })?;

    let user = User {
        username: register_form.username.clone(),
        password_hash,
        tasks: Vec::new(),
        quota: 36000,  // Set default quota 10hours
        used_quota: 0,
    };
    
    // Save user to Redis
    redis_service.save_user(&user).await
        .map_err(|e| {
            tracing::error!("Failed to save user {}: {}", user.username, e);
            AppError::Redis(e)
        })?;
    
    // Log successful registration
    tracing::info!("User registered successfully: {}", user.username);
    Ok(Redirect::to("/?error=Registration%20successful!%20Please%20login").into_response())
}

#[axum::debug_handler]
pub async fn handle_logout(
    session: Session,
) -> AppResult<Response> {
    // Get username before removing session
    let username = session.get::<String>("user_session")
        .await
        .map_err(|e| {
            tracing::error!("Session error: {}", e);
            AppError::Auth(format!("Failed to get session: {}", e))
        })?;

    // Remove session with proper error handling
    session.remove::<String>("user_session")
        .await
        .map_err(|e| {
            tracing::error!("Session removal error: {}", e);
            AppError::Auth(format!("Failed to remove session: {}", e))
        })?;

    if let Some(username) = username {
        tracing::info!("User '{}' logged out successfully", username);
    } else {
        tracing::warn!("Logout called with no active session");
    }

    Ok(Redirect::to("/").into_response())
}