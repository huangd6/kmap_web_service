use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use super::forms::ProcessForm;

// Define task status enum
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum TaskStatus {
    Queued,
    Processing,
    Completed,
    Failed,
}

#[derive(Serialize, Deserialize)]
pub struct TaskInfo {
    pub task_id: String,
    pub user: String,
    pub fasta_path: String,
    pub filename: String,
    pub status: TaskStatus,
    pub params: ProcessForm,
    pub result: Option<HashMap<String, u32>>,
    pub result_path: String,
    pub submission_time: DateTime<Utc>,
    pub completion_time: Option<DateTime<Utc>>,
}