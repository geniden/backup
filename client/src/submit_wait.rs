//! Wait for server `task_queued` ack after `run_task` (scheduler submit path).

use std::collections::HashMap;

use once_cell::sync::Lazy;
use tokio::sync::{Mutex, oneshot};

static WAITERS: Lazy<Mutex<HashMap<String, oneshot::Sender<anyhow::Result<()>>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub async fn register(def_id: &str) -> oneshot::Receiver<anyhow::Result<()>> {
    let (tx, rx) = oneshot::channel();
    WAITERS.lock().await.insert(def_id.to_string(), tx);
    rx
}

pub async fn ack(def_id: &str) {
    if let Some(tx) = WAITERS.lock().await.remove(def_id) {
        let _ = tx.send(Ok(()));
    }
}

pub async fn reject(def_id: &str, message: &str) {
    if let Some(tx) = WAITERS.lock().await.remove(def_id) {
        let _ = tx.send(Err(anyhow::anyhow!("{message}")));
    }
}
