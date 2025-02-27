use axum::{
    extract::{Multipart, State, Path, multipart::Field},
    response::{Html, IntoResponse, Response, Json},
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
//use std::process::Command;
use serde_json::json;
use crate::models::{TaskInfo, TaskStatus, ProcessForm};
use crate::services::RedisService;
use crate::errors::{AppError, AppResult};
use tracing;
use crate::config::Config;

pub async fn serve_upload_page() -> AppResult<Response> {
    tracing::info!("Serving upload page");
    
    let upload_html = fs::read_to_string("templates/upload.html")
        .map_err(|e| {
            tracing::error!("Failed to read upload template: {}", e);
            AppError::File(e)
        })?;
    
    tracing::debug!("Successfully loaded upload template");
    Ok(Html(upload_html).into_response())
}

// Helper struct to hold form data during file upload processing
struct UploadData {
    fasta_path: Option<String>,
    filename: Option<String>,
    form: ProcessForm,
}

pub async fn process_upload(
    State((redis_service, config)): State<(RedisService, Config)>,
    session: Session,
    mut multipart: Multipart,
) -> AppResult<Response> {
    // Get username from session with proper error handling
    let username = session
        .get::<String>("user_session")
        .await
        .map_err(|e| AppError::Auth(format!("Session error: {}", e)))?
        .ok_or_else(|| AppError::Auth("Not authenticated".into()))?;

    // Process multipart form
    let upload_data = process_multipart_form(&mut multipart, &username)
        .await
        .map_err(|e| AppError::Upload(format!("Error processing upload: {}", e)))?;

    // Create and queue task
    let task_id = create_and_queue_task(&redis_service, &username, upload_data)
        .await
        .map_err(|e| AppError::Task(format!("Error creating task: {}", e)))?;

    // Read template file
    let template = fs::read_to_string("templates/processing.html")
        .map_err(|e| AppError::File(e))?;

    // Return the response
    Ok(Html(template.replace("{{task_id}}", &task_id)).into_response())
}

// Helper function to process multipart form data from file upload
// Extracts file information and form parameters
async fn process_multipart_form(
    multipart: &mut Multipart,
    username: &str,
) -> AppResult<UploadData> {
    tracing::debug!("Processing multipart form for user: {}", username);
    
    let mut data = UploadData {
        fasta_path: None,
        filename: None,
        form: ProcessForm::default(),
    };

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        tracing::error!("Failed to get next field from multipart form: {}", e);
        AppError::Upload(format!("Failed to process form field: {}", e))
    })? {
        match field.name().unwrap_or("") {
            "fasta_file" => {
                let (path, name) = handle_file_upload(field, username).await?;
                data.fasta_path = Some(path);
                tracing::debug!("Processed file upload: {}", &name);
                data.filename = Some(name);
            }
            "n_trial" => {
                data.form.n_trial = parse_field_value(field).await?;
                tracing::debug!("Processed n_trial: {}", data.form.n_trial);
            }
            "top_k" => {
                data.form.top_k = parse_field_value(field).await?;
                tracing::debug!("Processed top_k: {}", data.form.top_k);
            }
            "revcom_mode" => {
                data.form.revcom_mode = parse_bool_field(field).await?;
                tracing::debug!("Processed revcom_mode: {}", data.form.revcom_mode);
            }
            "min_ham_dist_mode" => {
                data.form.min_ham_dist_mode = parse_bool_field(field).await?;
                tracing::debug!("Processed min_ham_dist_mode: {}", data.form.min_ham_dist_mode);
            }
            field_name => {
                tracing::warn!("Unexpected form field: {}", field_name);
            }
        }
    }

    // Validate required file was uploaded
    if data.fasta_path.is_none() {
        tracing::error!("No FASTA file was uploaded");
        return Err(AppError::Upload("No FASTA file uploaded".into()));
    }

    tracing::debug!("Successfully processed multipart form for user: {}", username);
    Ok(data)
}

// Helper function to handle file upload process
// Saves the uploaded file and returns its path and filename
async fn handle_file_upload(
    mut field: Field<'_>,
    username: &str,
) -> AppResult<(String, String)> {
    // Get filename with better error handling
    let filename = field
        .file_name()
        .ok_or_else(|| AppError::Upload("Missing filename in upload".into()))?
        .to_string();

    // Create temporary file
    let temp_path = create_temp_file(username, &filename)
        .map_err(|e| AppError::Upload(format!("Failed to create temporary file: {}", e)))?;

    // Save the uploaded file
    save_uploaded_file(&mut field, &temp_path)
        .await
        .map_err(|e| AppError::Upload(format!("Failed to save uploaded file: {}", e)))?;

    tracing::debug!("Successfully handled file upload: {} -> {}", filename, temp_path);
    Ok((temp_path, filename))
}

// Helper function to create and queue a new task
// Creates task info and updates Redis with new task
async fn create_and_queue_task(
    redis_service: &RedisService,
    username: &str,
    upload_data: UploadData,
) -> AppResult<String> {
    tracing::debug!("Creating and queueing task for user: {}", username);
    
    let task_id = uuid::Uuid::new_v4().to_string();
    
    // Clone the filename before unwrap and add error handling
    let filename = upload_data.filename.clone()
        .ok_or_else(|| AppError::Task("Missing filename in upload data".into()))?;
    
    // Create result directories - no need to map_err since it already returns AppResult
    let result_path = create_result_directories(username, &filename)?;

    // Get fasta path with error handling
    let fasta_path = upload_data.fasta_path
        .ok_or_else(|| AppError::Task("Missing FASTA file path in upload data".into()))?;

    // Create task info
    let task_info = TaskInfo {
        task_id: task_id.clone(),
        user: username.to_string(),
        fasta_path,
        filename,
        status: TaskStatus::Queued,
        params: upload_data.form,
        result: None,
        result_path,
        submission_time: Utc::now(),
        completion_time: None,
    };

    // Update user and queue task - no need to map_err since it already returns AppResult
    update_user_and_queue_task(redis_service, username, &task_id, &task_info).await?;

    tracing::debug!("Successfully created and queued task: {}", task_id);
    Ok(task_id)
}

pub async fn get_task_status(
    Path(task_id): Path<String>,
    State((redis_service, _)): State<(RedisService, Config)>,
) -> AppResult<Response> {
    tracing::debug!("Checking status for task: {}", task_id);

    let task = redis_service
        .get_task(&task_id)
        .await?
        .ok_or_else(|| {
            tracing::warn!("Task not found: {}", task_id);
            AppError::Task(format!("Task {} not found", task_id))
        })?;

    tracing::debug!("Task {} status: {:?}", task_id, task.status);

    let response = json!({
        "task_id": task.task_id,
        "status": task.status,
        "result": task.result,
        "filename": task.filename,
        "submit_time": task.submission_time,
        "complete_time": task.completion_time
    });

    tracing::trace!("Sending task status response: {:?}", response);
    Ok(Json(response).into_response())
}

pub async fn download_results(
    Path(task_id): Path<String>,
    State((redis_service, _)): State<(RedisService, Config)>,
) -> AppResult<Response> {
    tracing::info!("Starting download for task_id: {}", task_id);
    
    // Get task info using RedisService
    let task = redis_service
        .get_task(&task_id)
        .await?
        .ok_or_else(|| {
            tracing::warn!("Task not found: {}", task_id);
            AppError::Task(format!("Task {} not found", task_id))
        })?;
    
    tracing::debug!("Found task, checking result path: {}", task.result_path);
    let zip_path = format!("{}.zip", task.result_path);
    
    // Check if result directory exists
    if !std::path::Path::new(&task.result_path).exists() {
        tracing::error!("Result directory does not exist: {}", task.result_path);
        return Err(AppError::File(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Result directory not found: {}", task.result_path)
        )));
    }

    // Create zip file and get its size
    let file_size = create_zip_archive(&task.result_path, &zip_path).await?;
    
    tracing::debug!("Opening zip file for streaming");
    let file = File::open(&zip_path).await
        .map_err(|e| {
            tracing::error!("Failed to open zip file: {}", e);
            AppError::File(e)
        })?;
    
    tracing::debug!("Preparing to send file: {} (size: {} bytes)", task.filename, file_size);

    // Create buffered reader and stream
    let reader = BufReader::new(file);
    // Convert to stream for chunk-by-chunk reading
    let stream = ReaderStream::new(reader);
    // Create HTTP response body from stream
    let body = Body::from_stream(stream);

    let filename = format!("results_{}.zip", task.filename);
    
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
            tracing::error!("Failed to build response: {}", e);
            AppError::Task(format!("Failed to build download response: {}", e))
        })?;

    // Calculate a more appropriate timeout based on file size
    // Assume a conservative download speed of 1MB/s
    let timeout_secs = (file_size / (1024 * 1024) + 30) as u64;  // Add 30 seconds buffer
    let zip_path_clone = zip_path.clone();
    
    tracing::debug!("Setting cleanup timeout to {} seconds", timeout_secs);
    
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(timeout_secs)).await;
        if let Err(e) = tokio::fs::remove_file(&zip_path_clone).await {
            tracing::error!("Failed to clean up zip file {}: {}", zip_path_clone, e);
        } else {
            tracing::info!("Successfully cleaned up zip file: {}", zip_path_clone);
        }
    });

    tracing::info!("Successfully prepared download response for task: {}", task_id);
    Ok(response)
}

// Helper function to create a temporary file path
// Creates user-specific temp directory and generates unique filename
fn create_temp_file(username: &str, filename: &str) -> AppResult<String> {
    tracing::debug!("Creating temporary file for user: {}", username);
    
    // Create user-specific temp directory only if it doesn't exist
    let user_temp_dir = format!("temp/{}", username);
    if !std::path::Path::new(&user_temp_dir).exists() {
        std::fs::create_dir_all(&user_temp_dir).map_err(|e| {
            tracing::error!("Failed to create temp directory {}: {}", user_temp_dir, e);
            AppError::File(e)
        })?;
    }
    
    // Create temp file path with timestamp to avoid collisions
    let timestamp = chrono::Utc::now().timestamp();
    let temp_filename = format!("{}_{}", timestamp, filename);
    let temp_path = format!("{}/{}", user_temp_dir, temp_filename);
    
    tracing::debug!("Created temporary file path: {}", temp_path);
    Ok(temp_path)
}

// Helper function to save uploaded file chunks
// Writes file data to disk using buffered writer
async fn save_uploaded_file(
    field: &mut Field<'_>,
    temp_path: &str,
) -> AppResult<()> {
    tracing::debug!("Starting to save uploaded file to: {}", temp_path);
    
    // Create file with buffered writer
    let file = std::fs::File::create(temp_path).map_err(|e| {
        tracing::error!("Failed to create file {}: {}", temp_path, e);
        AppError::File(e)
    })?;
    let mut writer = std::io::BufWriter::new(file);
    
    // Read and write chunks
    while let Ok(Some(chunk)) = field.chunk().await {
        writer.write_all(&chunk).map_err(|e| {
            tracing::error!("Error writing chunk to {}: {}", temp_path, e);
            AppError::File(e)
        })?;
    }
    
    // Ensure all data is written
    writer.flush().map_err(|e| {
        tracing::error!("Error flushing file {}: {}", temp_path, e);
        AppError::File(e)
    })?;
    
    tracing::debug!("Successfully saved uploaded file to: {}", temp_path);
    Ok(())
}

// Helper function to create result directories
// Creates user-specific result directory with timestamp
fn create_result_directories(username: &str, filename: &str) -> AppResult<String> {
    tracing::debug!("Creating result directories for user: {}", username);
    
    // Create base results directory for user only if it doesn't exist
    let user_result_dir = format!("results/{}", username);
    if !std::path::Path::new(&user_result_dir).exists() {
        std::fs::create_dir_all(&user_result_dir).map_err(|e| {
            tracing::error!("Failed to create user result directory {}: {}", user_result_dir, e);
            AppError::File(e)
        })?;
    }
    
    // Get base name from filename
    let base_name = FilePath::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    
    // Create unique directory for this task using timestamp
    let timestamp = chrono::Utc::now().timestamp();
    let result_path = format!("{}/{}_{}", user_result_dir, base_name, timestamp);
    
    std::fs::create_dir_all(&result_path).map_err(|e| {
        tracing::error!("Failed to create task result directory {}: {}", result_path, e);
        AppError::File(e)
    })?;
    
    tracing::debug!("Successfully created result directory: {}", result_path);
    Ok(result_path)
}

// Helper function to update user data and queue task
// Updates user's task list and saves task info in Redis
async fn update_user_and_queue_task(
    redis_service: &RedisService,
    username: &str,
    task_id: &str,
    task_info: &TaskInfo,
) -> AppResult<()> {
    tracing::debug!("Updating user data and queueing task for user: {}", username);
    
    // Get current user data
    let mut user = redis_service.get_user(username).await?
        .ok_or_else(|| AppError::Task(format!("User {} not found", username)))?;
    
    // Add task to user's task list
    user.tasks.push(task_id.to_string());
    
    // Save updated user data
    redis_service.save_user(&user).await
        .map_err(|e| {
            tracing::error!("Failed to update user data for {}: {}", username, e);
            AppError::Redis(e)
        })?;
    
    // Save task info
    redis_service.save_task(task_info).await
        .map_err(|e| {
            tracing::error!("Failed to save task {}: {}", task_id, e);
            AppError::Redis(e)
        })?;
    
    // Queue task for processing
    redis_service.queue_task(task_info).await
        .map_err(|e| {
            tracing::error!("Failed to queue task {}: {}", task_id, e);
            AppError::Redis(e)
        })?;
    
    tracing::debug!("Successfully updated user data and queued task: {}", task_id);
    Ok(())
}

// Helper function to parse form field values
// Generic function to parse form fields into specified types
async fn parse_field_value<T>(
    field: Field<'_>,
) -> AppResult<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let value = field.text().await
        .map_err(|e| AppError::Upload(format!("Failed to read field: {}", e)))?;
    
    value.parse()
        .map_err(|e| AppError::Upload(format!(
            "Failed to parse field value '{}': {}", 
            value, e
        )))
}

// Helper function to parse boolean form fields
// Converts "true"/"false" strings to boolean values
async fn parse_bool_field(
    field: Field<'_>,
) -> AppResult<bool> {
    let value = field.text().await
        .map_err(|e| AppError::Upload(format!("Failed to read boolean field: {}", e)))?;
    
    match value.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(AppError::Upload(format!(
            "Invalid boolean value '{}', expected 'true' or 'false'", 
            value
        ))),
    }
}

// Helper function for creating zip archives
// Creates a zip file from source directory and returns its size
async fn create_zip_archive(source_path: &str, zip_path: &str) -> AppResult<u64> {
    tracing::debug!("Creating zip archive from {} to {}", source_path, zip_path);
    
    // Ensure source directory exists before attempting to zip
    if !std::path::Path::new(source_path).exists() {
        return Err(AppError::File(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Source directory not found: {}", source_path)
        )));
    }

    // Use tokio::process::Command for async execution
    let output = tokio::process::Command::new("zip")
        .arg("-rq")  // recursive and quiet mode
        .arg(zip_path)
        .arg(source_path)  
        .output()
        .await
        .map_err(|e| {
            tracing::error!("Failed to execute zip command: {}", e);
            AppError::Task(format!("Failed to create zip file: {}", e))
        })?;

    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        tracing::error!("Zip command failed: {}", error);
        return Err(AppError::Task(format!("Zip creation failed: {}", error)));
    }

    // Verify zip file was created and get its size
    let metadata = tokio::fs::metadata(zip_path).await.map_err(|e| {
        tracing::error!("Failed to verify zip file: {}", e);
        AppError::File(e)
    })?;

    let file_size = metadata.len();
    if file_size == 0 {
        tracing::error!("Created zip file is empty: {}", zip_path);
        return Err(AppError::Task("Created zip file is empty".into()));
    }

    tracing::debug!("Successfully created zip archive: {} (size: {} bytes)", zip_path, file_size);
    Ok(file_size)
} 

