use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct User {
    pub username: String,
    pub password_hash: String,  // We'll store hashed passwords, not plain text
    pub tasks: Vec<String>,     // List of task IDs
    pub quota: u64,            // Longest processing time
    pub used_quota: u64,       // Current usage time
}