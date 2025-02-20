use redis::{Client, AsyncCommands};
use std::sync::Arc;
use crate::models::{User, TaskInfo};

pub struct RedisService {
    client: Arc<Client>,
}

impl RedisService {
    pub fn new(client: Arc<Client>) -> Self {
        Self { client }
    }

    pub async fn get_user(&self, username: &str) -> Result<Option<User>, redis::RedisError> {
        let mut conn = self.client.get_async_connection().await?;
        let user_data: Option<String> = conn.get(format!("user:{}", username)).await?;
        Ok(user_data.map(|data| serde_json::from_str(&data).unwrap()))
    }

    pub async fn save_user(&self, user: &User) -> Result<(), redis::RedisError> {
        let mut conn = self.client.get_async_connection().await?;
        conn.set(
            format!("user:{}", user.username),
            serde_json::to_string(user).unwrap()
        ).await
    }

    pub async fn get_task(&self, task_id: &str) -> Result<Option<TaskInfo>, redis::RedisError> {
        let mut conn = self.client.get_async_connection().await?;
        let task_data: Option<String> = conn.get(format!("task:{}", task_id)).await?;
        Ok(task_data.map(|data| serde_json::from_str(&data).unwrap()))
    }

    pub async fn save_task(&self, task: &TaskInfo) -> Result<(), redis::RedisError> {
        let mut conn = self.client.get_async_connection().await?;
        conn.set(
            format!("task:{}", task.task_id),
            serde_json::to_string(task).unwrap()
        ).await
    }

    pub async fn queue_task(&self, task: &TaskInfo) -> Result<(), redis::RedisError> {
        let mut conn = self.client.get_async_connection().await?;
        conn.lpush("task_queue", serde_json::to_string(task).unwrap()).await
    }

    pub async fn pop_task(&self) -> Result<Option<TaskInfo>, redis::RedisError> {
        let mut conn = self.client.get_async_connection().await?;
        
        // Try to pop a task from the queue
        if let Some(task_json) = conn.rpop::<_, Option<String>>("task_queue", None).await? {
            // Parse the JSON into TaskInfo
            let task = serde_json::from_str(&task_json)
                .map_err(|e| redis::RedisError::from((redis::ErrorKind::TypeError, "Failed to parse task", e.to_string())))?;
            Ok(Some(task))
        } else {
            Ok(None)
        }
    }

    pub async fn delete_task(&self, task_id: &str) -> Result<(), redis::RedisError> {
        let mut conn = self.client.get_async_connection().await?;
        let task_key = format!("task:{}", task_id);
        conn.del(&task_key).await
    }
}

impl Clone for RedisService {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone()
        }
    }
} 