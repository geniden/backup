//! Tasks submitted to the server but not yet settled (completed/failed on client).

use std::collections::HashMap;

use once_cell::sync::Lazy;
use tokio::sync::Mutex;

use crate::models::connection::Connection;
use crate::models::task::Task;

#[derive(Clone, Debug)]
pub struct PendingWork {
    pub conn: Connection,
    pub task: Task,
}

static PENDING: Lazy<Mutex<HashMap<String, HashMap<String, PendingWork>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub async fn register(conn: Connection, task: Task) {
    PENDING
        .lock()
        .await
        .entry(conn.url.clone())
        .or_default()
        .insert(task.id.clone(), PendingWork { conn, task });
}

pub async fn remove(url: &str, def_id: &str) -> Option<PendingWork> {
    let mut guard = PENDING.lock().await;
    let entry = guard.get_mut(url)?;
    entry.remove(def_id)
}

pub async fn has_any(url: &str) -> bool {
    PENDING
        .lock()
        .await
        .get(url)
        .is_some_and(|m| !m.is_empty())
}

pub async fn snapshot(url: &str) -> Vec<PendingWork> {
    PENDING
        .lock()
        .await
        .get(url)
        .map(|m| m.values().cloned().collect())
        .unwrap_or_default()
}

pub async fn clear_url(url: &str) -> Vec<PendingWork> {
    PENDING
        .lock()
        .await
        .remove(url)
        .map(|m| m.into_values().collect())
        .unwrap_or_default()
}

pub async fn task_names(url: &str) -> Vec<String> {
    PENDING
        .lock()
        .await
        .get(url)
        .map(|m| m.values().map(|w| w.task.task_name.clone()).collect())
        .unwrap_or_default()
}
