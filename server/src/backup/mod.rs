//! Backup task implementations and TaskResult.

pub mod db_dump;
pub mod dir_sync;
pub mod encrypt;
pub mod file_archive;
pub mod naming;
pub mod shell;
pub mod zip_entries;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub filename: String,
    pub size_bytes: i64,
    pub files_count: u64,
    pub file_hash: String,
}

#[derive(Debug, Clone)]
pub enum TaskOutcome {
    File(TaskResult),
    /// Incremental task finished with nothing to deliver (optional detail for logs/UI).
    NoChanges {
        detail: Option<&'static str>,
    },
}