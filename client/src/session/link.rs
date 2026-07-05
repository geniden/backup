//! WebSocket connect / ensure for scheduler sessions and recovery.

use std::sync::Arc;

use once_cell::sync::Lazy;

use crate::manager::ConnectionManager;
use crate::models::connection::Connection;

use super::pump;
use super::URL_SESSIONS;

static CONNECTION_MANAGER: Lazy<ConnectionManager> = Lazy::new(ConnectionManager::new);

pub type SharedSplitSink = crate::manager::SharedSplitSink;

/// Open a session if needed; returns the active write half for this VPS URL.
pub async fn ensure_connected(
    conn: &Connection,
    pool: &sqlx::SqlitePool,
) -> anyhow::Result<SharedSplitSink> {
    {
        let sessions = URL_SESSIONS.lock().await;
        if let Some(inner) = sessions.get(&conn.url) {
            let needs_sync = inner.conn.lock().await.id != conn.id;
            *inner.conn.lock().await = conn.clone();
            inner.disconnect_task.lock().await.take().map(|h| h.abort());
            let sink = Arc::clone(&inner.write);
            drop(sessions);
            if needs_sync {
                resync_tasks(conn, pool, &sink).await?;
            }
            return Ok(sink);
        }
    }

    let inner = pump::establish(conn.clone(), pool.clone(), &CONNECTION_MANAGER).await?;
    URL_SESSIONS
        .lock()
        .await
        .insert(conn.url.clone(), Arc::clone(&inner));
    Ok(Arc::clone(&inner.write))
}

async fn resync_tasks(
    conn: &Connection,
    pool: &sqlx::SqlitePool,
    sink: &SharedSplitSink,
) -> anyhow::Result<()> {
    let tasks = crate::db::list_tasks_for_connection(pool, &conn.id).await?;
    let enabled: Vec<_> = tasks.into_iter().filter(|t| t.enabled).collect();
    crate::protocol::send_sync_tasks(conn, &enabled, sink).await
}

/// Called when the inbound pump stops; starts recovery if the drop was unexpected.
pub fn notify_pump_stopped(intentional: bool, url: String, pool: sqlx::SqlitePool) {
    if intentional {
        return;
    }
    tokio::spawn(async move {
        if super::pending::has_any(&url).await {
            super::recovery::run_recovery(&url, &pool).await;
        }
    });
}
