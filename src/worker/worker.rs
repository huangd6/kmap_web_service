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

pub async fn worker_process(
    redis_service: RedisService,
    semaphore: Arc<Semaphore>,
) {
    println!("Worker started");
    loop {
        if let Ok(Some(task)) = redis_service.pop_task().await {
            let _permit = semaphore.acquire().await.unwrap();
            
            // Update task status
            update_task_status(&redis_service, &task.task_id, TaskStatus::Processing).await;
            
            // Get user's remaining quota
            let remaining_quota = if let Ok(Some(user)) = redis_service.get_user(&task.user).await {
                let quota = (user.quota - user.used_quota).max(0) as u64;
                println!("Remaining quota for user {}: {} seconds", task.user, quota);
                quota
            } else {
                eprintln!("User {} not found", task.user);
                0
            };
            
            // Execute task
            match process_task_with_timeout(&task, remaining_quota).await {
                Ok(result) => {
                    println!("Task completed: {}", &task.task_id);
                    update_task_result_user_quota(&redis_service, &task.task_id, 
                        TaskStatus::Completed, Some(result)).await;
                }
                Err(e) => {
                    println!("Task failed: {}, error: {}", &task.task_id, &e);
                    update_task_result_user_quota(&redis_service, &task.task_id, 
                        TaskStatus::Failed, None).await;
                }
            }
        }
        
        sleep(Duration::from_secs(1)).await;
    }
}

// Process task with timeout
async fn process_task_with_timeout(
    task: &TaskInfo,
    remaining_quota: u64,
) -> Result<HashMap<String, u32>, Box<dyn std::error::Error + Send + Sync>> {
    let task_path = task.fasta_path.clone();
    let task_params = task.params.clone();
    let result_path = task.result_path.clone();

    // Spawn the task in a separate task to catch panics
    let result = tokio::time::timeout(
        Duration::from_secs(remaining_quota),
        tokio::spawn(async move {
            match process_task(
                Path::new(&task_path), 
                &task_params, 
                Path::new(&result_path)
            ).await {
                Ok(result) => Ok(result),
                Err(e) => Err(e),
            }
        })
    ).await;

    // Handle all possible error cases
    match result {
        Ok(spawn_result) => {
            match spawn_result {
                Ok(task_result) => task_result,
                Err(e) => {
                    // Handle panic in the spawned task
                    Err(format!("Task panicked: {}", e).into())
                }
            }
        }
        Err(_timeout_error) => {
            Err("Task timed out".into())
        }
    }
}

async fn process_task(
    fasta_path: &std::path::Path, 
    _form: &ProcessForm,
    result_path: &std::path::Path,
) -> Result<HashMap<String, u32>, Box<dyn std::error::Error + Send + Sync>> {
    // Check if file exists first
    if !fasta_path.exists() {
        return Err(format!("Error: FASTA file not found at path: {}", fasta_path.display()).into());
    }

    // Load FASTA file and convert to sequence vector
    let sequences = load_fasta(fasta_path.to_str().unwrap());

    // Calculate k-mers
    let kmer_length = 8;
    let kmer_counts = count_kmers_in_sequences(&sequences, kmer_length, false);
    
    // Convert HashMap to vector for sorting
    let mut kmer_counts_vec: Vec<_> = kmer_counts.into_iter().collect();
    // Sort by count in descending order
    kmer_counts_vec.sort_by(|a, b| b.1.cmp(&a.1));  
    
    let result: HashMap<String, u32> = kmer_counts_vec.iter()
        .take(10)
        .map(|(kmer, count)| {
            let kmer_string = String::from_utf8(hash2kmer(*kmer, kmer_length))
                .unwrap_or_else(|_| "Invalid UTF-8".to_string());
            (kmer_string, *count)
        })
        .collect();

    // Save results to file
    save_results_to_file(&kmer_counts_vec, kmer_length, result_path.to_str().unwrap())?;

    Ok(result)
}

fn save_results_to_file(
    kmer_counts_vec: &[(u64, u32)],
    kmer_length: usize,
    result_path: &str
) -> Result<()> {
    let output_path = Path::new(result_path).join("top10kmers.txt");
    
    let mut file = File::create(&output_path)
        .context(format!("Failed to create file: {}", output_path.display()))?;
    
    writeln!(file, "Top 10 k-mers and their counts:")
        .context("Failed to write header")?;
    
    for (kmer, count) in kmer_counts_vec.iter().take(10) {
        let kmer_string = String::from_utf8(hash2kmer(*kmer, kmer_length))
            .unwrap_or_else(|_| "Invalid UTF-8".to_string());
        writeln!(file, "{}: {}", kmer_string, count)
            .context(format!("Failed to write k-mer: {}", kmer_string))?;
    }
    
    Ok(())
}

pub async fn update_task_status(
    redis_service: &RedisService,
    task_id: &str,
    status: TaskStatus,
) {
    if let Ok(Some(mut task)) = redis_service.get_task(task_id).await {
        let status_debug = status.clone();
        task.status = status;
        
        // Update task in Redis
        if let Err(e) = redis_service.save_task(&task).await {
            eprintln!("Error updating task {} status: {}", task_id, e);
        } else {
            println!("Updated task {} status to {:?}", task_id, status_debug);
        }
    } else {
        eprintln!("Task {} not found in Redis", task_id);
    }
}

pub async fn update_task_result_user_quota(
    redis_service: &RedisService,
    task_id: &str,
    status: TaskStatus,
    result: Option<HashMap<String, u32>>,
) {
    if let Ok(Some(mut task)) = redis_service.get_task(task_id).await {
        task.status = status;
        task.result = result;
        
        // Update completion time when task is completed
        if matches!(task.status, TaskStatus::Completed | TaskStatus::Failed) {
            task.completion_time = Some(Utc::now());
            
            // Update user's quota usage
            if let Ok(Some(mut user)) = redis_service.get_user(&task.user).await {
                // Calculate time difference in seconds
                if let Some(completion_time) = task.completion_time {
                    let duration = completion_time.signed_duration_since(task.submission_time);
                    let seconds = duration.num_seconds() as u64;
                    user.used_quota += seconds;
                    
                    // Save updated user data
                    if let Err(e) = redis_service.save_user(&user).await {
                        eprintln!("Error updating user quota: {}", e);
                    } else {
                        println!("Updated quota for user {}: {} seconds used", task.user, seconds);
                    }
                }
            }
        }
        
        // Update task in Redis
        if let Err(e) = redis_service.save_task(&task).await {
            eprintln!("Error updating task with result: {}", e);
        }
    }
}