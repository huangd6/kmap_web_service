use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Response, Redirect},
};
use tower_sessions::Session;
use tokio::fs::remove_dir_all;
use crate::models::{User, TaskInfo};
use crate::services::RedisService;
use crate::errors::{AppError, AppResult};
use tracing;
use crate::config::Config;

pub async fn serve_user_dashboard(
    State((redis_service, config)): State<(RedisService, Config)>,
    session: Session,
) -> AppResult<Response> {
    tracing::info!("Accessing user dashboard");

    // Get username from session
    let username = session
        .get::<String>("user_session")
        .await
        .map_err(|e| AppError::Auth(format!("Session error: {}", e)))?
        .ok_or_else(|| AppError::Auth("Not authenticated".into()))?;

    tracing::info!("Found user in session: {}", username);

    // Get user data using RedisService
    let user = redis_service
        .get_user(&username)
        .await?
        .ok_or_else(|| AppError::Auth("User not found".into()))?;

    tracing::debug!("User tasks: {:?}", user.tasks);
    
    // Get task information for each task ID
    let mut tasks_info = Vec::new();
    for task_id in &user.tasks {
        match redis_service.get_task(task_id).await {
            Ok(Some(task)) => tasks_info.push(task),
            Ok(None) => tracing::warn!("Task {} not found for user {}", task_id, username),
            Err(e) => tracing::error!("Failed to fetch task {}: {}", task_id, e),
        }
    }
    
    // Sort tasks by submission time (newest first)
    tasks_info.sort_by(|a, b| b.submission_time.cmp(&a.submission_time));
    
    // Read and render the template
    let dashboard_html = std::fs::read_to_string("templates/user_dashboard.html")
        .map_err(|e| {
            tracing::error!("Failed to read dashboard template: {}", e);
            AppError::File(e)
        })?;
    
    // Replace template variables
    let tasks_html = tasks_info.iter().map(|task| {
        format!(
            r#"<tr>
                <td>{}</td>
                <td>{}</td>
                <td>{}</td>
                <td>{:?}</td>
                <td class="action-cell">
                    <a href="/process/{}" class="view-btn">View Results</a>
                    <a href="/delete/{}" class="delete-btn">Delete</a>
                </td>
            </tr>"#,
            task.filename,
            task.submission_time.format("%Y-%m-%d %H:%M:%S"),
            task.completion_time.map_or("Pending".to_string(), |t| t.format("%Y-%m-%d %H:%M:%S").to_string()),
            task.status,
            task.task_id,
            task.task_id
        )
    }).collect::<Vec<_>>().join("\n");
    
    let dashboard_html = dashboard_html
        .replace("{{username}}", &username)
        .replace("{{tasks}}", &tasks_html)
        .replace("{{quota_used}}", &user.used_quota.to_string())
        .replace("{{quota_total}}", &user.quota.to_string())
        .replace("{{task_count}}", &user.tasks.len().to_string())
        .replace("{{max_tasks}}", &config.user.max_tasks_per_user.to_string());
    
    tracing::info!("Successfully rendered dashboard for user: {}", username);
    Ok(Html(dashboard_html).into_response())
}

pub async fn view_process(
    State((redis_service, _)): State<(RedisService, Config)>,
    Path(task_id): Path<String>,
) -> AppResult<Response> {
    tracing::info!("Viewing process for task: {}", task_id);

    // Verify task exists before showing the processing page
    let task = redis_service
        .get_task(&task_id)
        .await?
        .ok_or_else(|| AppError::Task(format!("Task {} not found", task_id)))?;

    tracing::debug!("Found task with status: {:?}", task.status);

    // Read the template file
    let template = std::fs::read_to_string("templates/processing.html")
        .map_err(|e| {
            tracing::error!("Failed to read processing template: {}", e);
            AppError::File(e)
        })?;

    // Replace task ID in template
    let html = template.replace("{{task_id}}", &task_id);
    
    tracing::info!("Successfully rendered processing page for task: {}", task_id);
    Ok(Html(html).into_response())
}

pub async fn delete_task(
    State((redis_service, _)): State<(RedisService, Config)>,
    session: Session,
    Path(task_id): Path<String>,
) -> AppResult<Response> {
    // Get username from session
    let username = session
        .get::<String>("user_session")
        .await
        .map_err(|e| AppError::Auth(format!("Session error: {}", e)))?
        .ok_or_else(|| AppError::Auth("Not authenticated".into()))?;

    tracing::info!("Attempting to delete task {} for user {}", task_id, username);

    // Get user data using RedisService
    let mut user = redis_service
        .get_user(&username)
        .await?  // RedisError automatically converts to AppError::Redis
        .ok_or_else(|| AppError::Auth("User not found".into()))?;

    // Remove task from user's task list
    user.tasks.retain(|t| t != &task_id);
    
    // Update user data in Redis
    redis_service
        .save_user(&user)
        .await
        .map_err(|e| {
            tracing::error!("Failed to update user data: {}", e);
            AppError::Redis(e)
        })?;
    
    // Get task data to find result path
    if let Some(task_info) = redis_service.get_task(&task_id).await? {
        // Delete result files if they exist
        let result_path = task_info.result_path;
        if !result_path.is_empty() {
            remove_dir_all(&result_path)
                .await
                .map_err(|e| {
                    tracing::warn!("Failed to delete result directory {}: {}", result_path, e);
                    AppError::File(e)
                })?;
        }
    }
    
    // Delete task data from Redis
    redis_service
        .delete_task(&task_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete task data from Redis: {}", e);
            AppError::Redis(e)
        })?;

    tracing::info!("Successfully deleted task {} for user {}", task_id, username);
    
    // Redirect back to user dashboard
    Ok(Redirect::to("/user").into_response())
} 