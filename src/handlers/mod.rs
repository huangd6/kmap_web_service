mod auth;
mod task;
mod dashboard;

pub use auth::{serve_login_page, handle_login, handle_register, handle_logout};
pub use task::{serve_upload_page, process_upload, get_task_status, download_results};
pub use dashboard::{serve_user_dashboard, view_process, delete_task}; 