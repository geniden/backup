//! One row in task_runs: status, file size, anomalies for monitor/history.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TaskRun {
    pub id: i64,
    pub task_id: String,
    pub run_at: DateTime<Utc>,
    pub status: String,
    pub file_size_bytes: Option<i64>,
    pub file_path: Option<String>,
    pub error: Option<String>,
    pub anomaly_flags: Option<String>,
    pub server_run_id: Option<String>,
}
