//! In-memory task definitions synced from client.

use std::collections::HashMap;

use tokio::sync::Mutex;

#[derive(Clone, Debug)]
pub struct TaskDefinition {
    pub def_id: String,
    pub task_name: String,
    pub task_type: String,
    pub data: serde_json::Value,
}

#[derive(Default)]
pub struct TaskRegistry {
    inner: Mutex<HashMap<String, HashMap<String, TaskDefinition>>>,
}

impl TaskRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn replace_connection(&self, connection_id: &str, tasks: Vec<TaskDefinition>) {
        let map: HashMap<String, TaskDefinition> = tasks
            .into_iter()
            .map(|t| (t.def_id.clone(), t))
            .collect();
        self.inner
            .lock()
            .await
            .insert(connection_id.to_string(), map);
    }

    pub async fn remove_connection(&self, connection_id: &str) {
        self.inner.lock().await.remove(connection_id);
    }

    pub async fn get(&self, connection_id: &str, def_id: &str) -> Option<TaskDefinition> {
        self.inner
            .lock()
            .await
            .get(connection_id)?
            .get(def_id)
            .cloned()
    }
}
