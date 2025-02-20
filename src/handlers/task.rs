use axum::{
    extract::{Multipart, State, Path, multipart::Field},
    response::{Html, IntoResponse, Response, Json, Redirect},
    http::{StatusCode, header},
    body::Body,
};
use tower_sessions::Session;
use std::{path::Path as FilePath, fs, io::Write};
use chrono::Utc;
use tokio::{
    fs::File,
    io::BufReader,
};
use tokio_util::io::ReaderStream;
use std::process::Command;
use serde_json::json;
use crate::models::{TaskInfo, TaskStatus, ProcessForm, User};
use crate::services::RedisService;

pub async fn serve_upload_page() -> impl IntoResponse {
    let upload_html = fs::read_to_string("templates/upload.html")
        .unwrap_or_else(|_| "Error loading upload page".to_string());
    Html(upload_html)
}

// Helper struct to hold form data
struct UploadData {
    fasta_path: Option<String>,
    filename: Option<String>,
    form: ProcessForm,
}

pub async fn process_upload(
    State(redis_service): State<RedisService>,
    session: Session,
    mut multipart: Multipart,
) -> Response {
    // Get username from session directly
    let username = match session.get::<String>("user_session").await {
        Ok(Some(username)) => username,
        _ => return Redirect::to("/").into_response(),
    };

    // Process multipart form
    let upload_data = match process_multipart_form(&mut multipart, &username).await {
        Ok(data) => data,
        Err(e) => return Html(format!("Error processing upload: {}", e)).into_response(),
    };

    // Create and queue task
    match create_and_queue_task(&redis_service, &username, upload_data).await {
        Ok(task_id) => {
            match fs::read_to_string("templates/processing.html") {
                Ok(template) => Html(template.replace("{{task_id}}", &task_id)).into_response(),
                Err(e) => Html(format!("Template error: {}", e)).into_response(),
            }
        }
        Err(e) => Html(format!("Error creating task: {}", e)).into_response(),
    }
}


async fn process_multipart_form(
    multipart: &mut Multipart,
    username: &str,
) -> Result<UploadData, String> {
    let mut data = UploadData {
        fasta_path: None,
        filename: None,
        form: ProcessForm::default(),
    };

    while let Some(field) = multipart.next_field().await
        .map_err(|e| e.to_string())? 
    {
        match field.name().unwrap_or("") {
            "fasta_file" => {
                let (path, name) = handle_file_upload(field, username).await?;
                data.fasta_path = Some(path);
                data.filename = Some(name);
            }
            "n_trial" => {
                data.form.n_trial = parse_field_value(field).await?;
            }
            "top_k" => {
                data.form.top_k = parse_field_value(field).await?;
            }
            "revcom_mode" => {
                data.form.revcom_mode = parse_bool_field(field).await?;
            }
            "min_ham_dist_mode" => {
                data.form.min_ham_dist_mode = parse_bool_field(field).await?;
            }
            _ => {}
        }
    }

    if data.fasta_path.is_none() {
        return Err("No FASTA file uploaded".to_string());
    }

    Ok(data)
}

async fn handle_file_upload(
    mut field: Field<'_>,
    username: &str,
) -> Result<(String, String), String> {
    let filename = field.file_name()
        .unwrap_or("unknown")
        .to_string();

    let temp_path = create_temp_file(username, &filename)?;
    save_uploaded_file(&mut field, &temp_path).await?;

    Ok((temp_path, filename))
}

async fn create_and_queue_task(
    redis_service: &RedisService,
    username: &str,
    upload_data: UploadData,
) -> Result<String, String> {
    let task_id = uuid::Uuid::new_v4().to_string();
    
    // Clone the filename before first unwrap
    let filename = upload_data.filename.clone().unwrap();
    let result_path = create_result_directories(username, &filename)?;

    let task_info = TaskInfo {
        task_id: task_id.clone(),
        user: username.to_string(),
        fasta_path: upload_data.fasta_path.unwrap(),
        filename,  // Use the cloned filename
        status: TaskStatus::Queued,
        params: upload_data.form,
        result: None,
        result_path,
        submission_time: Utc::now(),
        completion_time: None,
    };

    update_user_and_queue_task(redis_service, username, &task_id, &task_info).await?;
    Ok(task_id)
}

pub async fn get_task_status(
    Path(task_id): Path<String>,
    State(redis_service): State<RedisService>,
) -> impl IntoResponse {
    match redis_service.get_task(&task_id).await {
        Ok(Some(task)) => {
            println!("Parsed task status: {:?}", task.status);
            Json(json!({
                "task_id": task.task_id,
                "status": task.status,
                "result": task.result,
                "filename": task.filename,
                "submit_time": task.submission_time,
                "complete_time": task.completion_time
            })).into_response()
        }
        Ok(None) => {
            Json(json!({
                "error": "Task not found"
            })).into_response()
        }
        Err(e) => {
            eprintln!("Redis error: {}", e);
            Json(json!({
                "error": "Error retrieving task information"
            })).into_response()
        }
    }
}

pub async fn download_results(
    Path(task_id): Path<String>,
    State(redis_service): State<RedisService>,
) -> Result<Response<Body>, (StatusCode, String)> {
    // Add debug logging
    println!("Starting download for task_id: {}", task_id);
    
    // Get task info using RedisService
    let task = redis_service.get_task(&task_id).await
        .map_err(|e| {
            println!("Redis error: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?
        .ok_or_else(|| {
            println!("Task not found: {}", task_id);
            (StatusCode::NOT_FOUND, "Task not found".to_string())
        })?;
    
    // Add debug logging for paths
    println!("Result path: {}", task.result_path);
    let zip_path = format!("{}.zip", task.result_path);
    println!("Zip path: {}", zip_path);

    // Check if result directory exists
    if !std::path::Path::new(&task.result_path).exists() {
        println!("Result directory does not exist: {}", task.result_path);
        return Err((StatusCode::NOT_FOUND, "Result directory not found".to_string()));
    }

    // Create zip file with verbose output
    let output = Command::new("zip")
        .arg("-rv") // Added verbose flag
        .arg(&zip_path)
        .arg(&task.result_path)
        .output()
        .map_err(|e| {
            println!("Failed to execute zip command: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        println!("Zip command failed: {}", error);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create zip file: {}", error),
        ));
    }

    // Check if zip file was created
    if !std::path::Path::new(&zip_path).exists() {
        println!("Zip file was not created: {}", zip_path);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Zip file was not created".to_string(),
        ));
    }

    let file = File::open(&zip_path).await
        .map_err(|e| {
            println!("Failed to open zip file: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;
    
    let file_size = file.metadata().await
        .map_err(|e| {
            println!("Failed to get file metadata: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?
        .len();

    // Create buffered reader for efficient reading
    let reader = BufReader::new(file);
    // Convert to stream for chunk-by-chunk reading
    let stream = ReaderStream::new(reader);
    // Create HTTP response body from stream
    let body = Body::from_stream(stream);

    let filename = format!("results_{}.zip", task.filename);
    println!("Sending file: {} (size: {} bytes)", filename, file_size);
    
    // Building the HTTP response
    let response = Response::builder()
        // Set HTTP status code to 200 OK
        .status(StatusCode::OK)
        // Tell browser this is a zip file
        .header(header::CONTENT_TYPE, "application/zip")
        // Tell browser to download file instead of displaying it
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", filename))
        // Tell browser the total file size
        .header(header::CONTENT_LENGTH, file_size.to_string())
        // Attach the streaming body we created earlier
        .body(body)
        // Handle any errors in building the response
        .map_err(|e| {
            println!("Failed to build response: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    // Calculate a more appropriate timeout based on file size
    // Assume a conservative download speed of 1MB/s
    let timeout_secs = (file_size / (1024 * 1024) + 30) as u64;  // Add 30 seconds buffer
    println!("Setting cleanup timeout to {} seconds for file size {} bytes", timeout_secs, file_size);

    // Cleanup code with dynamic timeout
    let zip_path_clone = zip_path.clone();
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(timeout_secs)).await;
        if let Err(e) = tokio::fs::remove_file(&zip_path_clone).await {
            eprintln!("Failed to clean up zip file {}: {}", zip_path_clone, e);
        } else {
            println!("Successfully cleaned up zip file: {}", zip_path_clone);
        }
    });

    println!("Download response prepared successfully");
    Ok(response)
}

fn create_temp_file(username: &str, filename: &str) -> Result<String, String> {
    // Create user-specific temp directory only if it doesn't exist
    let user_temp_dir = format!("temp/{}", username);
    if !std::path::Path::new(&user_temp_dir).exists() {
        std::fs::create_dir_all(&user_temp_dir)
            .map_err(|e| format!("Failed to create temp directory: {}", e))?;
    }
    
    // Create temp file path with timestamp to avoid collisions
    let timestamp = chrono::Utc::now().timestamp();
    let temp_filename = format!("{}_{}", timestamp, filename);
    let temp_path = format!("{}/{}", user_temp_dir, temp_filename);
    
    Ok(temp_path)
}

async fn save_uploaded_file(
    field: &mut Field<'_>,
    temp_path: &str,
) -> Result<(), String> {
    // Create file with buffered writer
    let file = std::fs::File::create(temp_path)
        .map_err(|e| format!("Failed to create file: {}", e))?;
    let mut writer = std::io::BufWriter::new(file);
    
    while let Ok(Some(chunk)) = field.chunk().await {
        writer.write_all(&chunk)
            .map_err(|e| format!("Error writing chunk: {}", e))?;
    }
    
    // Ensure all data is written
    writer.flush()
        .map_err(|e| format!("Error flushing file: {}", e))?;
    
    Ok(())
}

fn create_result_directories(username: &str, filename: &str) -> Result<String, String> {
    // Create base results directory for user only if it doesn't exist
    let user_result_dir = format!("results/{}", username);
    if !std::path::Path::new(&user_result_dir).exists() {
        std::fs::create_dir_all(&user_result_dir)
            .map_err(|e| format!("Failed to create user result directory: {}", e))?;
    }
    
    // Create unique directory for this task using timestamp
    let timestamp = chrono::Utc::now().timestamp();
    let base_name = FilePath::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    
    let result_path = format!("{}/{}_{}", user_result_dir, base_name, timestamp);
    std::fs::create_dir_all(&result_path)
        .map_err(|e| format!("Failed to create task result directory: {}", e))?;
    
    Ok(result_path)
}

async fn update_user_and_queue_task(
    redis_service: &RedisService,
    username: &str,
    task_id: &str,
    task_info: &TaskInfo,
) -> Result<(), String> {
    // Get current user data
    let mut user = redis_service.get_user(username).await
        .map_err(|e| format!("Redis error: {}", e))?
        .ok_or_else(|| "User not found".to_string())?;
    
    // Add task to user's task list
    user.tasks.push(task_id.to_string());
    
    // Save updated user data
    redis_service.save_user(&user).await
        .map_err(|e| format!("Failed to update user data: {}", e))?;
    
    // Save task info
    redis_service.save_task(task_info).await
        .map_err(|e| format!("Failed to save task: {}", e))?;
    
    // Queue task for processing
    redis_service.queue_task(task_info).await
        .map_err(|e| format!("Failed to queue task: {}", e))?;
    
    Ok(())
}

async fn parse_field_value<T>(
    field: Field<'_>,
) -> Result<T, String>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let value = field.text().await
        .map_err(|e| format!("Failed to read field: {}", e))?;
    
    value.parse()
        .map_err(|e| format!("Failed to parse value: {}", e))
}

async fn parse_bool_field(
    field: Field<'_>,
) -> Result<bool, String> {
    let value = field.text().await
        .map_err(|e| format!("Failed to read field: {}", e))?;
    
    match value.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("Invalid boolean value: {}", value)),
    }
} 

