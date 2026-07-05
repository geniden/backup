//! Connect-on-demand WebSocket sessions for the scheduler.

mod link;
mod pending;
mod pump;
mod recovery;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use once_cell::sync::Lazy;
use tokio::sync::Mutex;
use tracing::{debug, warn};

use crate::cron;
use crate::i18n;
use crate::manager::SharedSplitSink;
use crate::models::connection::Connection;
use crate::models::task::Task;

pub use link::ensure_connected;

pub(crate) static URL_SESSIONS: Lazy<Mutex<HashMap<String, Arc<UrlSessionInner>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub const SESSION_HOLD: Duration = Duration::from_secs(10 * 60);

pub(crate) const SESSION_HOLD_MINUTES: &str = "10";

/// Grace period after last task before evaluating session disconnect.
pub(crate) const DISCONNECT_GRACE: Duration = Duration::from_secs(3);

pub struct UrlSessionInner {
    pub url: String,
    pub conn: Mutex<Connection>,
    pub write: SharedSplitSink,
    pub stop: tokio::sync::watch::Sender<bool>,
    pub in_flight: AtomicU32,
    pub disconnect_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

pub async fn register_pending(conn: Connection, task: Task) {
    pending::register(conn.clone(), task).await;
    if let Some(inner) = URL_SESSIONS.lock().await.get(&conn.url) {
        inner.in_flight.fetch_add(1, Ordering::SeqCst);
        inner.disconnect_task.lock().await.take().map(|h| h.abort());
    }
}

pub async fn task_settled(url: &str, def_id: &str) {
    if pending::remove(url, def_id).await.is_some() {
        in_flight_decrement(url).await;
    }
}

pub(crate) async fn reset_in_flight(url: &str) {
    if let Some(inner) = URL_SESSIONS.lock().await.get(url) {
        inner.in_flight.store(0, Ordering::SeqCst);
    }
}

async fn in_flight_decrement(url: &str) {
    if let Some(inner) = URL_SESSIONS.lock().await.get(url) {
        if inner.in_flight.load(Ordering::SeqCst) > 0 {
            inner.in_flight.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

pub(super) async fn work_finished(
    conn: &Connection,
    pool: &sqlx::SqlitePool,
    inner: &Arc<UrlSessionInner>,
) {
    pump::schedule_disconnect_after_idle(conn.clone(), pool.clone(), Arc::clone(inner)).await;
}

pub async fn warm_if_needed(conn: &Connection, pool: &sqlx::SqlitePool) {
    match cron::next_task_within_for_connection(pool, &conn.id, SESSION_HOLD).await {
        Ok(true) => {
            if let Err(e) = ensure_connected(conn, pool).await {
                warn!("[{}] Session warm-up failed: {}", conn.slug, e);
            }
        }
        Ok(false) => {
            debug!(
                "{}",
                i18n::t_fmt("session.idle_until_task", &[("slug", &conn.slug)]),
            );
        }
        Err(e) => {
            warn!("[{}] Could not evaluate next task: {}", conn.slug, e);
        }
    }
}
