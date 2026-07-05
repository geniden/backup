//! Maps server run_id to client task while a backup is in flight.

use once_cell::sync::Lazy;
use std::collections::{HashMap, VecDeque};
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct PendingRun {
    pub task_id: String,
    pub task_type: String,
}

static PENDING_BY_RUN_ID: Lazy<Mutex<HashMap<String, PendingRun>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// FIFO fallback for servers that omit def_id in task_queued.
static LEGACY_PENDING_BY_URL: Lazy<Mutex<HashMap<String, VecDeque<PendingRun>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub async fn register_legacy_pending(url: &str, pending: PendingRun) {
    LEGACY_PENDING_BY_URL
        .lock()
        .await
        .entry(url.to_string())
        .or_default()
        .push_back(pending);
}

pub async fn map_run(server_run_id: &str, pending: PendingRun) {
    PENDING_BY_RUN_ID
        .lock()
        .await
        .insert(server_run_id.to_string(), pending);
}

pub async fn on_task_queued_legacy(url: &str, server_run_id: &str) {
    if let Some(pending) = LEGACY_PENDING_BY_URL
        .lock()
        .await
        .get_mut(url)
        .and_then(|q| q.pop_front())
    {
        map_run(server_run_id, pending).await;
    }
}

pub async fn take_by_run_id(server_run_id: &str) -> Option<PendingRun> {
    PENDING_BY_RUN_ID
        .lock()
        .await
        .remove(server_run_id)
}

pub fn pending_from_task(task: &crate::models::task::Task) -> PendingRun {
    PendingRun {
        task_id: task.id.clone(),
        task_type: task.task_type.clone(),
    }
}
