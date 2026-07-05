//! Manual single-task run (test without scheduler).

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use tokio::time::timeout;
use tracing::info;

use crate::db;
use crate::models::connection::Connection;
use crate::models::task::Task;
use crate::protocol;
use crate::ws_inbound::InboundPump;

fn noop_on_disconnect() -> Arc<dyn Fn() + Send + Sync> {
    Arc::new(|| {})
}

pub async fn run_task_now(conn: &Connection, task: &Task) -> anyhow::Result<()> {
    info!(
        "[{}] Manual run: {} ({})",
        conn.slug, task.task_name, task.task_type
    );

    let pool = db::init_db().await?;

    let (ws, _) = crate::tls::connect_ws(conn).await?;
    let (write, mut read) = ws.split();
    let write = Arc::new(tokio::sync::Mutex::new(write));

    protocol::authenticate_and_sync(conn, &pool, &mut read, &write).await?;

    protocol::send_run_task(conn, task, &write).await?;

    let mut pump = InboundPump::start(read, Arc::clone(&write), noop_on_disconnect());

    let deadline = tokio::time::Instant::now() + Duration::from_secs(600);

    let run_result: anyhow::Result<()> = async {
        loop {
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("Manual run timed out");
            }

            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let text = match timeout(remaining, pump.recv()).await {
                Ok(Some(text)) => text,
                Ok(None) => anyhow::bail!("Connection closed"),
                Err(_) => anyhow::bail!("Manual run timed out"),
            };

            if text.contains("task_failed") {
                let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                let error = json["error"].as_str().unwrap_or("unknown error");
                anyhow::bail!("Task failed: {error}");
            }

            protocol::handle_server_message(conn, &text, Some(&write), Some(&pool), None).await?;

            if text.contains("task_completed") {
                return Ok(());
            }
        }
    }
    .await;

    pump.shutdown(&write).await;
    run_result
}

pub async fn reset_dir_sync_now(conn: &Connection, task: &Task) -> anyhow::Result<()> {
    info!(
        "[{}] dir_sync reset: {} ({})",
        conn.slug, task.task_name, task.id
    );

    let pool = db::init_db().await?;

    let (ws, _) = crate::tls::connect_ws(conn).await?;
    let (write, mut read) = ws.split();
    let write = Arc::new(tokio::sync::Mutex::new(write));

    protocol::authenticate_and_sync(conn, &pool, &mut read, &write).await?;
    protocol::send_reset_dir_sync(conn, task, &write).await?;

    let mut pump = InboundPump::start(read, Arc::clone(&write), noop_on_disconnect());

    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);

    let reset_result: anyhow::Result<()> = async {
        loop {
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("dir_sync reset timed out");
            }

            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let text = match timeout(remaining, pump.recv()).await {
                Ok(Some(text)) => text,
                Ok(None) => anyhow::bail!("Connection closed"),
                Err(_) => anyhow::bail!("dir_sync reset timed out"),
            };

            if text.contains("dir_sync_reset_ok") {
                return Ok(());
            }

            if text.contains("\"type\":\"error\"") {
                let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                let error = json["message"].as_str().unwrap_or("unknown error");
                anyhow::bail!("{error}");
            }

            protocol::handle_server_message(conn, &text, Some(&write), Some(&pool), None).await?;
        }
    }
    .await;

    pump.shutdown(&write).await;
    reset_result?;

    crate::dir_sync::reset_local_state(&pool, &task.id).await?;
    Ok(())
}
