use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Response, Redirect},
};
use tower_sessions::Session;
use tokio::fs::remove_dir_all;
use crate::models::{User, TaskInfo};
use crate::services::RedisService;

pub async fn serve_user_dashboard(
    State(redis_service): State<RedisService>,
    session: Session,
) -> Response {
    println!("Accessing dashboard...");

    // Get username from session
    let username = match session.get::<String>("user_session").await {
        Ok(Some(username)) => {
            println!("Found user in session: {}", username);
            username
        },
        _ => return Redirect::to("/").into_response(),
    };

    // Get user data using RedisService
    match redis_service.get_user(&username).await {
        Ok(Some(user)) => {
            println!("User tasks: {:?}", user.tasks);
            
            // Get task information for each task ID
            let mut tasks_info = Vec::new();
            for task_id in &user.tasks {
                if let Ok(Some(task)) = redis_service.get_task(task_id).await {
                    tasks_info.push(task);
                }
            }
            
            // Sort tasks by submission time (newest first)
            tasks_info.sort_by(|a, b| b.submission_time.cmp(&a.submission_time));
            
            // Read and render the template
            let dashboard_html = std::fs::read_to_string("templates/user_dashboard.html")
                .unwrap_or_else(|_| "Error loading dashboard page".to_string());
            
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
                .replace("{{quota_total}}", &user.quota.to_string());
            
            Html(dashboard_html).into_response()
        }
        Ok(None) => {
            println!("User not found: {}", username);
            Html("<p>User not found</p>").into_response()
        }
        Err(e) => {
            println!("Redis error: {}", e);
            Html("<p>Server error</p>").into_response()
        }
    }
}

pub async fn view_process(
    State(_redis_service): State<RedisService>,
    Path(task_id): Path<String>,
) -> Response {
    let template = std::fs::read_to_string("templates/processing.html")
        .unwrap_or_else(|_| "Error loading processing page".to_string());
    let template = template.replace("{{task_id}}", &task_id);
    Html(template).into_response()
}

pub async fn delete_task(
    State(redis_service): State<RedisService>,
    session: Session,
    Path(task_id): Path<String>,
) -> Response {
    // Get username from session
    let username = match session.get::<String>("user_session").await {
        Ok(Some(username)) => username,
        _ => return Redirect::to("/").into_response(),
    };

    // Get user data using RedisService
    if let Ok(Some(mut user)) = redis_service.get_user(&username).await {
        // Remove task from user's task list
        user.tasks.retain(|t| t != &task_id);
        
        // Update user data in Redis
        if let Err(e) = redis_service.save_user(&user).await {
            eprintln!("Failed to update user data: {}", e);
            return Redirect::to("/user").into_response();
        }
        
        // Get task data to find result path
        if let Ok(Some(task_info)) = redis_service.get_task(&task_id).await {
            // Delete result files if they exist
            let result_path = task_info.result_path;
            if !result_path.is_empty() {
                let _ = remove_dir_all(&result_path).await;
            }
        }
        
        // Delete task data from Redis
        if let Err(e) = redis_service.delete_task(&task_id).await {
            eprintln!("Failed to delete task data: {}", e);
        }
    }
    
    // Redirect back to user dashboard
    Redirect::to("/user").into_response()
} 