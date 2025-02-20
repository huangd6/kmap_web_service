use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub redis: RedisConfig,
    pub worker: WorkerConfig,
    pub upload: UploadConfig,
    pub user: UserConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RedisConfig {
    pub url: String,
    pub sentinel_enabled: bool,
    pub sentinel_url: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct WorkerConfig {
    pub worker_count: usize,
    pub max_concurrent_tasks: usize,
}

#[derive(Debug, Deserialize, Clone)]
pub struct UploadConfig {
    pub max_file_size: usize,  // 10MB in bytes
    pub temp_dir: String,
    pub results_dir: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct UserConfig {
    pub default_quota: u64,  // in seconds
    pub max_tasks_per_user: usize,
}

impl Config {
    pub fn load() -> Result<Self, config::ConfigError> {
        let config = config::Config::builder()
            .add_source(config::File::with_name("config/default"))
            .add_source(config::Environment::with_prefix("APP"))
            .build()?;

        config.try_deserialize()
    }
}