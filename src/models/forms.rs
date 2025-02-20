use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct LoginForm {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct RegisterForm {
    pub username: String,
    pub password: String,
    pub confirm_password: String,
}

#[derive(Deserialize, Serialize, Clone, Default)]
pub struct ProcessForm {
    pub n_trial: u32,
    pub top_k: u32,
    pub revcom_mode: bool,
    pub min_ham_dist_mode: bool,
} 