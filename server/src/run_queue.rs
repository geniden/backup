//! Per-connection job queue (one active run at a time).

use std::collections::{HashSet, VecDeque};

use tokio::sync::Mutex;

#[derive(Clone, Debug)]
pub struct RunJob {
    pub run_id: String,
    pub def_id: String,
    pub connection_id: String,
}

#[derive(Default)]
pub struct RunQueue {
    inner: Mutex<VecDeque<RunJob>>,
}

impl RunQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn enqueue(&self, job: RunJob) {
        self.inner.lock().await.push_back(job);
    }

    pub async fn len(&self) -> usize {
        self.inner.lock().await.len()
    }

    pub async fn pop_ready(&self, busy: &HashSet<String>) -> Option<RunJob> {
        let mut queue = self.inner.lock().await;
        if queue.is_empty() {
            return None;
        }
        if busy.contains(&queue.front()?.connection_id) {
            return None;
        }
        queue.pop_front()
    }

    pub async fn cancel_for_connection(&self, connection_id: &str) -> Vec<String> {
        let mut queue = self.inner.lock().await;
        let mut cancelled = Vec::new();
        queue.retain(|job| {
            if job.connection_id == connection_id {
                cancelled.push(job.run_id.clone());
                false
            } else {
                true
            }
        });
        cancelled
    }
}
