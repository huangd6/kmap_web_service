use tokio::time::{sleep, Duration};
use std::path::Path;
use crate::models::{TaskInfo, TaskStatus, User, ProcessForm};
use std::fs;
use std::sync::Arc;
use tokio::sync::Semaphore;
use crate::kmap_algorithms::kmer_count::{load_fasta, count_kmers_in_sequences, hash2kmer};

use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use anyhow::{Result, Context};
use chrono::Utc;
use crate::services::RedisService;
use crate::errors::worker::{WorkerError, WorkerResult};
use tracing;

pub async fn worker_process(
    redis_service: RedisService,
    semaphore: Arc<Semaphore>,
) {
    tracing::info!("Worker started");
    
    loop {
        // First acquire the semaphore before popping a task
        let _permit = match semaphore.acquire().await {
            Ok(permit) => permit,
            Err(e) => {
                tracing::error!("Failed to acquire semaphore: {}", e);
                sleep(Duration::from_secs(1)).await;
                continue;
            }
        };

        // Try to get a task from the queue only after we have the semaphore
        match redis_service.pop_task().await {
            Ok(Some(task)) => {
                let task_id = task.task_id.clone();
                let username = task.user.clone();
                
                tracing::debug!("Processing task {} for user {}", task_id, username);
                
                // Use a closure to handle the task processing with proper cleanup
                let process_result = async {
                    // Update task status to Processing
                    update_task_status(&redis_service, &task_id, TaskStatus::Processing).await?;
                    
                    // Get user's remaining quota
                    let remaining_quota = get_user_quota(&redis_service, &username).await?;
                    
                    // Execute task
                    process_task_with_timeout(&task, remaining_quota).await
                }.await;

                // Handle the result of task processing
                match process_result {
                    Ok(result) => {
                        tracing::info!("Task {} completed successfully", task_id);
                        if let Err(e) = update_task_result_user_quota(
                            &redis_service,
                            &task_id,
                            TaskStatus::Completed,
                            Some(result)
                        ).await {
                            tracing::error!("Failed to update task result: {}", e);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Task {} failed: {}", task_id, e);
                        if let Err(update_err) = update_task_result_user_quota(
                            &redis_service,
                            &task_id,
                            TaskStatus::Failed,
                            None
                        ).await {
                            tracing::error!("Failed to update task status after error: {}", update_err);
                        }
                    }
                }
            }
            Ok(None) => {
                // No tasks in queue, drop the permit and wait before checking again
                drop(_permit);
                sleep(Duration::from_secs(1)).await;
            }
            Err(e) => {
                tracing::error!("Failed to pop task from queue: {}", e);
                // Drop the permit and wait before retrying
                drop(_permit);
                sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

// Helper function to get user's remaining quota
async fn get_user_quota(redis_service: &RedisService, username: &str) -> WorkerResult<u64> {
    let user = redis_service
        .get_user(username)
        .await
        .map_err(WorkerError::Redis)?
        .ok_or_else(|| WorkerError::Processing(format!("User {} not found", username)))?;

    let remaining_quota = (user.quota - user.used_quota).max(0) as u64;
    tracing::debug!("Remaining quota for user {}: {} seconds", username, remaining_quota);

    if remaining_quota == 0 {
        return Err(WorkerError::QuotaExceeded(username.to_string()));
    }

    Ok(remaining_quota)
}

// Process task with timeout
async fn process_task_with_timeout(
    task: &TaskInfo,
    remaining_quota: u64,
) -> WorkerResult<HashMap<String, u32>> {
    let task_path = task.fasta_path.clone();
    let task_path_delete = task.fasta_path.clone();
    let task_params = task.params.clone();
    let result_path = task.result_path.clone();

    tracing::debug!(
        "Starting task processing with timeout of {} seconds",
        remaining_quota
    );

    // Spawn the task in a separate task to catch panics
    let result = tokio::time::timeout(
        Duration::from_secs(remaining_quota),
        tokio::spawn(async move {
            process_task(
                Path::new(&task_path), 
                &task_params, 
                Path::new(&result_path)
            ).await
        })
    ).await;

    // Delete the FASTA file after processing, regardless of the result
    if let Err(e) = tokio::fs::remove_file(&task_path_delete).await {
        // Map the IO error to a WorkerError
        return Err(WorkerError::Io(e));
    }
    tracing::info!("Successfully deleted FASTA file: {}", task_path_delete);

    // Handle all possible error cases
    match result {
        Ok(spawn_result) => {
            match spawn_result {
                Ok(task_result) => task_result,
                Err(e) => {
                    tracing::error!("Task panicked: {}", e);
                    Err(WorkerError::TaskPanic(e.to_string()))
                }
            }
        }
        Err(_timeout_error) => {
            tracing::error!("Task timed out after {} seconds", remaining_quota);
            Err(WorkerError::Timeout(remaining_quota))
        }
    }
}

async fn process_task(
    fasta_path: &std::path::Path, 
    _form: &ProcessForm,
    result_path: &std::path::Path,
) -> WorkerResult<HashMap<String, u32>> {
    // Check if file exists first
    if !fasta_path.exists() {
        tracing::error!("FASTA file not found: {}", fasta_path.display());
        return Err(WorkerError::FileNotFound(
            fasta_path.display().to_string()
        ));
    }

    // Get file path as string with proper error handling
    let fasta_path_str = fasta_path.to_str()
        .ok_or_else(|| {
            tracing::error!("Invalid UTF-8 in file path: {}", fasta_path.display());
            WorkerError::Processing(format!(
                "Invalid UTF-8 in file path: {}", 
                fasta_path.display()
            ))
        })?;

    tracing::debug!("Loading FASTA file: {}", fasta_path_str);

    // Load FASTA file and convert to sequence vector
    let sequences = load_fasta(fasta_path_str);

    // Calculate k-mers
    let kmer_length = 8;
    tracing::debug!("Calculating {}-mers", kmer_length);
    let kmer_counts = count_kmers_in_sequences(&sequences, kmer_length, false);
    
    // Convert HashMap to vector for sorting
    let mut kmer_counts_vec: Vec<_> = kmer_counts.into_iter().collect();
    // Sort by count in descending order
    kmer_counts_vec.sort_by(|a, b| b.1.cmp(&a.1));  
    
    tracing::debug!("Converting top {} k-mers to strings", 10);
    let result: HashMap<String, u32> = kmer_counts_vec.iter()
        .take(10)
        .map(|(kmer, count)| {
            let kmer_string = String::from_utf8(hash2kmer(*kmer, kmer_length))
                .map_err(|e| {
                    tracing::error!("Invalid UTF-8 in k-mer conversion: {}", e);
                    WorkerError::InvalidKmer(format!(
                        "Failed to convert k-mer hash {} to string", 
                        kmer
                    ))
                })?;
            Ok((kmer_string, *count))
        })
        .collect::<Result<HashMap<String, u32>, WorkerError>>()?;

    // Get result path as string with proper error handling
    let result_path_str = result_path.to_str()
        .ok_or_else(|| {
            tracing::error!("Invalid UTF-8 in result path: {}", result_path.display());
            WorkerError::Processing(format!(
                "Invalid UTF-8 in result path: {}", 
                result_path.display()
            ))
        })?;

    // Save results to file
    tracing::debug!("Saving results to file: {}", result_path_str);
    save_results_to_file(&kmer_counts_vec, kmer_length, result_path_str)?;

    tracing::info!("Successfully processed task for file: {}", fasta_path_str);
    Ok(result)
}

fn save_results_to_file(
    kmer_counts_vec: &[(u64, u32)],
    kmer_length: usize,
    result_path: &str
) -> WorkerResult<()> {
    let output_path = Path::new(result_path).join("top10kmers.txt");
    
    // Create file with proper error handling
    let mut file = File::create(&output_path)
        .map_err(|e| {
            tracing::error!("Failed to create file {}: {}", output_path.display(), e);
            WorkerError::Io(e)
        })?;
    
    tracing::debug!("Created output file: {}", output_path.display());

    // Write header
    writeln!(file, "Top 10 k-mers and their counts:")
        .map_err(|e| {
            tracing::error!("Failed to write header: {}", e);
            WorkerError::Io(e)
        })?;

    // Write k-mer counts
    for (kmer, count) in kmer_counts_vec.iter().take(10) {
        let kmer_string = String::from_utf8(hash2kmer(*kmer, kmer_length))
            .map_err(|e| {
                tracing::error!("Invalid UTF-8 in k-mer: {}", e);
                WorkerError::InvalidKmer(format!("Failed to convert k-mer hash {} to string", kmer))
            })?;

        writeln!(file, "{}: {}", kmer_string, count)
            .map_err(|e| {
                tracing::error!("Failed to write k-mer {}: {}", kmer_string, e);
                WorkerError::Io(e)
            })?;

        tracing::trace!("Wrote k-mer: {} (count: {})", kmer_string, count);
    }
    
    tracing::info!("Successfully saved results to {}", output_path.display());
    Ok(())
}

pub async fn update_task_status(
    redis_service: &RedisService,
    task_id: &str,
    status: TaskStatus,
) -> WorkerResult<()> {
    // Get task with proper error handling
    let mut task = redis_service
        .get_task(task_id)
        .await
        .map_err(WorkerError::Redis)?
        .ok_or_else(|| WorkerError::Processing(format!("Task {} not found", task_id)))?;

    // Log the status update
    tracing::debug!(
        "Updating task {} status from {:?} to {:?}",
        task_id,
        task.status,
        status
    );

    // Update task status
    task.status = status.clone();  // Clone status since we need it for logging later

    // Save updated task
    redis_service
        .save_task(&task)
        .await
        .map_err(|e| {
            tracing::error!("Failed to save task {}: {}", task_id, e);
            WorkerError::Redis(e)
        })?;

    tracing::info!("Successfully updated task {} status to {:?}", task_id, status);
    Ok(())
}

pub async fn update_task_result_user_quota(
    redis_service: &RedisService,
    task_id: &str,
    status: TaskStatus,
    result: Option<HashMap<String, u32>>,
) -> WorkerResult<()> {
    // Get task with proper error handling
    let mut task = redis_service
        .get_task(task_id)
        .await
        .map_err(WorkerError::Redis)?
        .ok_or_else(|| WorkerError::Processing(format!("Task {} not found", task_id)))?;

    tracing::debug!(
        "Updating task {} result and status to {:?}",
        task_id,
        status
    );

    // Update task status and result
    let status_for_logging = status.clone();  // Store status for logging
    task.status = status;
    task.result = result;

    // Update completion time and user quota for completed or failed tasks
    if matches!(task.status, TaskStatus::Completed | TaskStatus::Failed) {
        task.completion_time = Some(Utc::now());

        // Get user data for quota update
        let mut user = redis_service
            .get_user(&task.user)
            .await
            .map_err(WorkerError::Redis)?
            .ok_or_else(|| WorkerError::Processing(format!("User {} not found", task.user)))?;

        // Calculate and update quota usage
        if let Some(completion_time) = task.completion_time {
            let duration = completion_time.signed_duration_since(task.submission_time);
            let seconds = duration.num_seconds() as u64;
            user.used_quota += seconds;

            // Save updated user data
            redis_service
                .save_user(&user)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to update quota for user {}: {}", task.user, e);
                    WorkerError::Redis(e)
                })?;

            tracing::info!(
                "Updated quota for user {}: {} seconds used",
                task.user,
                seconds
            );
        }
    }

    // Save updated task
    redis_service
        .save_task(&task)
        .await
        .map_err(|e| {
            tracing::error!("Failed to save task {}: {}", task_id, e);
            WorkerError::Redis(e)
        })?;

    tracing::info!(
        "Successfully updated task {} with status {:?}",
        task_id,
        status_for_logging
    );
    Ok(())
}