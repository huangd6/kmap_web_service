use axum::{
    extract::{Form, State},
    response::{Html, IntoResponse, Response, Redirect},
};
use tower_sessions::Session;
use std::fs;
use bcrypt::{hash, verify, DEFAULT_COST};
use crate::models::{LoginForm, RegisterForm, User};
use crate::services::RedisService;

pub async fn serve_login_page() -> impl IntoResponse {
    let login_html = fs::read_to_string("templates/login.html")
        .unwrap_or_else(|_| "Error loading login page".to_string());
    Html(login_html)
}

#[axum::debug_handler]
pub async fn handle_login(
    State(redis_service): State<RedisService>,
    session: Session,
    Form(login_form): Form<LoginForm>,
) -> Response {
    println!("Login attempt for user: {}", login_form.username);
    
    match redis_service.get_user(&login_form.username).await {
        Ok(Some(user)) => {
            if verify(&login_form.password, &user.password_hash).unwrap() {
                println!("Password verified for user: {}", login_form.username);
                if let Err(e) = session.insert("user_session", login_form.username).await {
                    println!("Session error: {}", e);
                    return Redirect::to("/?error=Server%20error").into_response();
                }
                Redirect::to("/user").into_response()
            } else {
                println!("Invalid password for user: {}", login_form.username);
                Redirect::to("/?error=Password%20is%20incorrect%2C%20please%20re-enter").into_response()
            }
        }
        Ok(None) => {
            println!("User not found: {}", login_form.username);
            Redirect::to("/?error=Username%20does%20not%20exist").into_response()
        }
        Err(e) => {
            eprintln!("Redis error: {}", e);
            Redirect::to("/?error=Server%20error").into_response()
        }
    }
}

pub async fn handle_register(
    State(redis_service): State<RedisService>,
    Form(register_form): Form<RegisterForm>,
) -> Response {
    if register_form.password != register_form.confirm_password {
        return Redirect::to("/?error=Passwords%20don't%20match&form=register").into_response();
    }
    
    // Check if user exists using RedisService
    if let Ok(Some(_)) = redis_service.get_user(&register_form.username).await {
        return Redirect::to("/?error=Username%20already%20taken&form=register").into_response();
    }
    
    // Create new user
    let password_hash = hash(register_form.password.as_bytes(), DEFAULT_COST).unwrap();
    let user = User {
        username: register_form.username,
        password_hash,
        tasks: Vec::new(),
        quota: 36000,  // Set default quota 10hours
        used_quota: 0,
    };
    
    // Save user using RedisService
    if let Err(e) = redis_service.save_user(&user).await {
        eprintln!("Failed to save user: {}", e);
        return Redirect::to("/?error=Registration%20failed&form=register").into_response();
    }
    
    // Only successful registration returns to login form
    Redirect::to("/?error=Registration%20successful!%20Please%20login").into_response()
}

#[axum::debug_handler]
pub async fn handle_logout(
    session: Session,
) -> Response {
    if let Err(e) = session.remove::<String>("user_session").await {
        println!("Session removal error: {}", e);
    }
    Redirect::to("/").into_response()
}