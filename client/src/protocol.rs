//! WebSocket protocol: auth, sync, run, download, task_runs.

use std::sync::Arc;

use futures::{SinkExt, StreamExt};
use tracing::error;

use crate::db;
use crate::i18n;
use crate::models::connection::Connection;
use crate::models::task::Task;
use crate::run_tracker;
use crate::session;
use crate::validation;

pub fn filename_from_download_url(url: &str) -> Option<String> {
    url.split('/')
        .next_back()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub async fn handle_server_message(
    conn: &Connection,
    text: &str,
    sink: Option<&crate::manager::SharedSplitSink>,
    pool: Option<&sqlx::SqlitePool>,
    on_settled: Option<Arc<dyn Fn(&Connection) + Send + Sync>>,
) -> anyhow::Result<()> {
    let json: serde_json::Value = serde_json::from_str(text)?;
    let msg_type = json["type"].as_str().unwrap_or("");

    match msg_type {
        "pong" if json["status"] == "authenticated" => {
            tracing::debug!("[{}] Server pong: authenticated", conn.slug);
        }
        "sync_ok" => {
            let count = json["count"].as_u64().unwrap_or(0);
            tracing::info!("[{}] Synced {} task(s) to server", conn.slug, count);
        }
        "dir_sync_reset_ok" => {
            tracing::info!(
                "[{}] dir_sync reset acknowledged by server",
                conn.slug
            );
        }
        "ping" => {
            tracing::debug!("[{}] Received ping", conn.slug);
        }
        "task_queued" => {
            let server_run_id = json["task_id"].as_str().unwrap_or("?");
            let queue_position = json["queue_position"].as_i64().unwrap_or(1);
            if queue_position > 1 {
                tracing::info!(
                    "{}",
                    i18n::t_fmt(
                        "protocol.task_queued_position",
                        &[
                            ("slug", &conn.slug),
                            ("run_id", server_run_id),
                            ("pos", &queue_position.to_string()),
                        ],
                    )
                );
            } else {
                tracing::info!(
                    "{}",
                    i18n::t_fmt(
                        "protocol.task_queued",
                        &[("slug", &conn.slug), ("run_id", server_run_id)],
                    )
                );
            }
            if let Some(def_id) = json.get("def_id").and_then(|v| v.as_str()) {
                crate::submit_wait::ack(def_id).await;
            }
            if let Some(pool) = pool {
                if let Some(def_id) = json.get("def_id").and_then(|v| v.as_str()) {
                    if let Ok(Some(task)) = db::get_task_by_id(pool, def_id).await {
                        run_tracker::map_run(
                            server_run_id,
                            run_tracker::pending_from_task(&task),
                        )
                        .await;
                    } else {
                        tracing::warn!(
                            "[{}] task_queued: unknown def_id {}",
                            conn.slug,
                            def_id
                        );
                    }
                } else {
                    run_tracker::on_task_queued_legacy(&conn.url, server_run_id).await;
                }
            } else {
                run_tracker::on_task_queued_legacy(&conn.url, server_run_id).await;
            }
        }
        "task_completed" => {
            let status = json["status"].as_str().unwrap_or("");
            let server_run_id = json["task_id"].as_str().unwrap_or("?");

            if status == "no_changes" {
                let detail = json.get("detail").and_then(|v| v.as_str()).unwrap_or("");
                let msg = match detail {
                    "empty" => i18n::t("task.dir_sync_empty_source"),
                    "up_to_date" => i18n::t("task.dir_sync_up_to_date"),
                    _ => i18n::t("task.dir_sync_no_changes"),
                };
                tracing::info!(
                    "[{}] Task {} completed — {}",
                    conn.slug,
                    server_run_id,
                    msg
                );
                if let Some(pool) = pool {
                    let pending =
                        resolve_pending(pool, conn, &json, server_run_id, None).await;
                    if let Some(pending) = pending {
                        db::insert_task_run(
                            pool,
                            &pending.task_id,
                            "ok",
                            Some(0),
                            None,
                            None,
                            None,
                            Some(server_run_id),
                        )
                        .await?;
                        db::update_last_run(pool, &pending.task_id).await?;
                    }
                }
                if let Some(def_id) = json.get("def_id").and_then(|v| v.as_str()) {
                    settle_task(conn, def_id, &on_settled).await;
                } else {
                    notify_settled(conn, &on_settled);
                }
                return Ok(());
            }

            if status != "success" {
                tracing::debug!(
                    "[{}] Task {} completed with status {}",
                    conn.slug,
                    server_run_id,
                    status
                );
                if let Some(def_id) = json.get("def_id").and_then(|v| v.as_str()) {
                    settle_task(conn, def_id, &on_settled).await;
                } else {
                    notify_settled(conn, &on_settled);
                }
                return Ok(());
            }

            let url = json["download_url"].as_str().unwrap_or("");
            let file_hash = json["file_hash"].as_str().unwrap_or("");
            tracing::info!(
                "[{}] Task {} completed, downloading",
                conn.slug,
                server_run_id
            );

            let pending = if let Some(pool) = pool {
                resolve_pending(pool, conn, &json, server_run_id, Some(url)).await
            } else {
                run_tracker::take_by_run_id(server_run_id).await
            };
            let db_pool = pool.ok_or_else(|| {
                anyhow::anyhow!("Internal error: database pool required for download")
            })?;
            let (file_path, file_size) =
                crate::download::download_backup(db_pool, conn, url, file_hash).await?;

            if let (Some(pool), Some(pending)) = (pool, pending) {
                record_successful_run(
                    pool,
                    &pending,
                    server_run_id,
                    file_size,
                    &file_path,
                )
                .await?;

                if let Err(e) =
                    crate::retention::maybe_run_after_success(pool, &conn.id, &conn.slug).await
                {
                    tracing::warn!("[{}] Retention: {}", conn.slug, e);
                }
            } else if pool.is_some() {
                tracing::warn!(
                    "[{}] Completed run {} not recorded: could not resolve task",
                    conn.slug,
                    server_run_id
                );
            }

            if let (Some(sink), Some(filename)) = (sink, filename_from_download_url(url)) {
                send_check_download(conn, &filename, sink).await?;
            }
            if let Some(def_id) = json.get("def_id").and_then(|v| v.as_str()) {
                settle_task(conn, def_id, &on_settled).await;
            } else {
                notify_settled(conn, &on_settled);
            }
        }
        "task_failed" => {
            let server_run_id = json["task_id"].as_str().unwrap_or("?");
            let error = json["error"].as_str().unwrap_or("unknown error");
            let def_id = json.get("def_id").and_then(|v| v.as_str()).unwrap_or("");
            let task_name = task_name_for_def(pool, def_id).await;

            error!(
                "{}",
                i18n::t_fmt(
                    "protocol.task_broken",
                    &[
                        ("slug", &conn.slug),
                        ("name", &task_name),
                        ("detail", error),
                    ],
                )
            );

            if let Some(pool) = pool {
                let pending =
                    resolve_pending(pool, conn, &json, server_run_id, None).await;
                if let Some(pending) = pending {
                    db::insert_task_run(
                        pool,
                        &pending.task_id,
                        "fail",
                        None,
                        None,
                        Some(error),
                        None,
                        Some(server_run_id),
                    )
                    .await?;
                }
            }
            if !def_id.is_empty() {
                settle_task(conn, def_id, &on_settled).await;
            } else {
                notify_settled(conn, &on_settled);
            }
        }
        "error" => {
            let message = json["message"].as_str().unwrap_or("unknown error");
            if let Some(def_id) = json.get("def_id").and_then(|v| v.as_str()) {
                crate::submit_wait::reject(def_id, message).await;
            }
            tracing::warn!(
                "{}",
                i18n::t_fmt(
                    "protocol.server_error",
                    &[("slug", &conn.slug), ("message", message)],
                )
            );
        }
        _ => tracing::debug!("[{}] Unhandled message type: {}", conn.slug, msg_type),
    }

    Ok(())
}

fn notify_settled(conn: &Connection, on_settled: &Option<Arc<dyn Fn(&Connection) + Send + Sync>>) {
    if let Some(cb) = on_settled {
        cb(conn);
    }
}

async fn settle_task(
    conn: &Connection,
    def_id: &str,
    on_settled: &Option<Arc<dyn Fn(&Connection) + Send + Sync>>,
) {
    session::task_settled(&conn.url, def_id).await;
    notify_settled(conn, on_settled);
}

async fn task_name_for_def(pool: Option<&sqlx::SqlitePool>, def_id: &str) -> String {
    if def_id.is_empty() {
        return "?".to_string();
    }
    if let Some(pool) = pool {
        if let Ok(Some(task)) = db::get_task_by_id(pool, def_id).await {
            return task.task_name;
        }
    }
    def_id.to_string()
}

async fn record_successful_run(
    pool: &sqlx::SqlitePool,
    pending: &run_tracker::PendingRun,
    server_run_id: &str,
    file_size: u64,
    file_path: &std::path::Path,
) -> anyhow::Result<()> {
    let prev_size = db::get_previous_ok_run_size(pool, &pending.task_id)
        .await?
        .map(|s| s as u64);
    let flags = crate::anomaly::detect(&pending.task_type, file_size, prev_size);
    let status = if flags.is_empty() { "ok" } else { "warn" };
    let flags_str = crate::anomaly::flags_to_string(&flags);

    db::insert_task_run(
        pool,
        &pending.task_id,
        status,
        Some(file_size as i64),
        Some(&file_path.display().to_string()),
        None,
        flags_str.as_deref(),
        Some(server_run_id),
    )
    .await?;
    db::update_last_run(pool, &pending.task_id).await?;
    Ok(())
}

async fn resolve_pending(
    pool: &sqlx::SqlitePool,
    conn: &Connection,
    json: &serde_json::Value,
    server_run_id: &str,
    download_url: Option<&str>,
) -> Option<run_tracker::PendingRun> {
    if let Some(pending) = run_tracker::take_by_run_id(server_run_id).await {
        return Some(pending);
    }
    if let Some(def_id) = json.get("def_id").and_then(|v| v.as_str()) {
        if let Ok(Some(task)) = db::get_task_by_id(pool, def_id).await {
            return Some(run_tracker::pending_from_task(&task));
        }
    }
    if let Some(url) = download_url {
        if let Some(filename) = filename_from_download_url(url) {
            return resolve_task_from_filename(pool, conn, &filename).await;
        }
    }
    None
}

async fn resolve_task_from_filename(
    pool: &sqlx::SqlitePool,
    conn: &Connection,
    filename: &str,
) -> Option<run_tracker::PendingRun> {
    let tasks = db::list_tasks_for_connection(pool, &conn.id).await.ok()?;

    if filename.starts_with("backup_") {
        for task in &tasks {
            let label = backup_file_label(&task.task_type);
            let prefix = format!("backup_{}_{}_", task.task_name, label);
            if filename.starts_with(&prefix) {
                return Some(run_tracker::pending_from_task(task));
            }
        }
    }

    // Legacy dir_sync names: sync_{task}_YYYY-MM-DD_HHMMSS.zip
    if filename.starts_with("sync_") {
        for task in &tasks {
            if validation::normalize_task_type(&task.task_type) != "dir_sync" {
                continue;
            }
            let prefix = format!("sync_{}_", task.task_name);
            if filename.starts_with(&prefix) {
                return Some(run_tracker::pending_from_task(task));
            }
        }
    }

    if filename.ends_with("_output.txt") {
        // Legacy shell reports (pre backup_{name}_shell_{date}_{time}.txt)
        let shell_tasks: Vec<_> = tasks
            .iter()
            .filter(|t| validation::normalize_task_type(&t.task_type) == "shell")
            .collect();
        if shell_tasks.len() == 1 {
            return Some(run_tracker::pending_from_task(shell_tasks[0]));
        }
        for task in shell_tasks {
            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&task.data_json) {
                let script = data
                    .get("script_name")
                    .or_else(|| data.get("script"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let stem = script.trim_end_matches(".sh");
                if !stem.is_empty() && filename.contains(&format!("_{stem}_output.txt")) {
                    return Some(run_tracker::pending_from_task(task));
                }
            }
        }
    }

    None
}

fn backup_file_label(task_type: &str) -> &str {
    match validation::normalize_task_type(task_type) {
        "mysql_dump" | "mariadb_dump" => "mysql",
        "postgresql_dump" => "postgresql",
        "sqlite_dump" => "sqlite",
        "files_archive" => "files",
        "dir_sync" => "sync",
        "shell" => "shell",
        other => other,
    }
}

pub async fn send_check_download(
    conn: &Connection,
    filename: &str,
    sink: &crate::manager::SharedSplitSink,
) -> anyhow::Result<()> {
    tracing::info!("[{}] Check download (delete on server): {}", conn.slug, filename);
    let message = serde_json::json!({
        "type": "check_download",
        "filename": filename,
    });
    sink.lock()
        .await
        .send(tokio_tungstenite::tungstenite::Message::Text(
            message.to_string(),
        ))
        .await?;
    Ok(())
}

pub async fn send_sync_tasks(
    conn: &Connection,
    tasks: &[Task],
    sink: &crate::manager::SharedSplitSink,
) -> anyhow::Result<()> {
    let items: Vec<serde_json::Value> = tasks
        .iter()
        .filter_map(|task| {
            let data: serde_json::Value = serde_json::from_str(&task.data_json).ok()?;
            let task_type = validation::normalize_task_type(&task.task_type);
            Some(serde_json::json!({
                "def_id": task.id,
                "task_name": task.task_name,
                "task_type": task_type,
                "data": data,
            }))
        })
        .collect();

    tracing::info!(
        "[{}] Syncing {} enabled task(s) to server",
        conn.slug,
        items.len()
    );

    let message = serde_json::json!({
        "type": "sync_tasks",
        "tasks": items,
    });

    sink.lock()
        .await
        .send(tokio_tungstenite::tungstenite::Message::Text(
            message.to_string(),
        ))
        .await?;

    Ok(())
}

pub async fn send_reset_dir_sync(
    conn: &Connection,
    task: &Task,
    sink: &crate::manager::SharedSplitSink,
) -> anyhow::Result<()> {
    tracing::info!(
        "[{}] Requesting dir_sync reset for '{}' ({})",
        conn.slug,
        task.task_name,
        task.id
    );

    let message = serde_json::json!({
        "type": "reset_dir_sync",
        "def_id": task.id,
    });

    sink.lock()
        .await
        .send(tokio_tungstenite::tungstenite::Message::Text(
            message.to_string(),
        ))
        .await?;

    Ok(())
}

pub async fn dispatch_run_task(
    conn: &Connection,
    task: &Task,
    sink: &crate::manager::SharedSplitSink,
) -> anyhow::Result<()> {
    tracing::info!(
        "[{}] Running task '{}' ({})",
        conn.slug,
        task.task_name,
        validation::normalize_task_type(&task.task_type)
    );

    let message = serde_json::json!({
        "type": "run_task",
        "def_id": task.id,
    });

    sink.lock()
        .await
        .send(tokio_tungstenite::tungstenite::Message::Text(
            message.to_string(),
        ))
        .await?;

    Ok(())
}

/// Send `run_task` and wait until the server acknowledges `task_queued` (or error).
pub async fn submit_run_task(
    conn: &Connection,
    task: &Task,
    sink: &crate::manager::SharedSplitSink,
) -> anyhow::Result<()> {
    let ack = crate::submit_wait::register(&task.id).await;
    dispatch_run_task(conn, task, sink).await?;

    match tokio::time::timeout(std::time::Duration::from_secs(45), ack).await {
        Ok(Ok(Ok(()))) => Ok(()),
        Ok(Ok(Err(e))) => Err(e),
        Ok(Err(_)) => anyhow::bail!("{}", i18n::t("protocol.submit_channel_closed")),
        Err(_) => anyhow::bail!("{}", i18n::t("protocol.submit_timeout")),
    }
}

/// Fire-and-forget alias used by manual run (pump handles responses).
pub async fn send_run_task(
    conn: &Connection,
    task: &Task,
    sink: &crate::manager::SharedSplitSink,
) -> anyhow::Result<()> {
    run_tracker::register_legacy_pending(&conn.url, run_tracker::pending_from_task(task)).await;
    dispatch_run_task(conn, task, sink).await
}

pub async fn wait_for_message_type(
    conn: &Connection,
    read: &mut futures::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    write: &crate::manager::SharedSplitSink,
    expected_type: &str,
    timeout: std::time::Duration,
    pool: Option<&sqlx::SqlitePool>,
) -> anyhow::Result<serde_json::Value> {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        if tokio::time::Instant::now() > deadline {
            anyhow::bail!("Timeout waiting for server message: {}", expected_type);
        }

        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let msg = tokio::time::timeout(remaining, read.next()).await;

        match msg {
            Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text)))) => {
                let json: serde_json::Value = serde_json::from_str(&text)?;
                if json.get("type").and_then(|v| v.as_str()) == Some("ping") {
                    write
                        .lock()
                        .await
                        .send(tokio_tungstenite::tungstenite::Message::Text(
                            r#"{"type":"pong"}"#.into(),
                        ))
                        .await?;
                    continue;
                }
                if json.get("type").and_then(|v| v.as_str()) == Some(expected_type) {
                    return Ok(json);
                }
                if json.get("type").and_then(|v| v.as_str()) == Some("error") {
                    let message = json["message"].as_str().unwrap_or("unknown error");
                    if message.contains("Device not authorized") || message.contains("device_id") {
                        anyhow::bail!("Device auth denied: {}", message);
                    }
                    anyhow::bail!("Server error: {}", message);
                }
                let _ = handle_server_message(conn, &text, Some(write), pool, None).await;
            }
            Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Ping(payload)))) => {
                write
                    .lock()
                    .await
                    .send(tokio_tungstenite::tungstenite::Message::Pong(payload))
                    .await?;
            }
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(e))) => {
                let msg = e.to_string();
                if msg.contains("close_notify") || msg.contains("Connection reset") {
                    anyhow::bail!(
                        "Connection closed by server — device_id may have been rejected (check server logs)"
                    );
                }
                anyhow::bail!("WebSocket error: {}", e);
            }
            Ok(None) => anyhow::bail!(
                "Connection closed by server — device_id may have been rejected (check server logs)"
            ),
            Err(_) => anyhow::bail!("Timeout waiting for server message: {}", expected_type),
        }
    }
}

pub fn device_id_short(id: &str) -> String {
    if id.len() >= 64 {
        format!("{}...{}", &id[..8], &id[56..])
    } else if id.len() > 16 {
        format!("{}...", &id[..16])
    } else {
        id.to_string()
    }
}

pub async fn authenticate_and_sync(
    conn: &Connection,
    pool: &sqlx::SqlitePool,
    read: &mut futures::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    write: &crate::manager::SharedSplitSink,
) -> anyhow::Result<()> {
    let device_id = crate::device_id::compute_device_id()?;
    tracing::info!(
        "[{}] Device auth: sending device_id {}",
        conn.slug,
        device_id_short(&device_id)
    );
    let auth = serde_json::json!({"type": "auth", "device_id": device_id});
    write
        .lock()
        .await
        .send(tokio_tungstenite::tungstenite::Message::Text(
            auth.to_string(),
        ))
        .await?;

    wait_for_message_type(
        conn,
        read,
        write,
        "pong",
        std::time::Duration::from_secs(15),
        Some(pool),
    )
    .await?;

    tracing::info!(
        "[{}] Device auth: server accepted ({})",
        conn.slug,
        device_id_short(&device_id)
    );

    let tasks = crate::db::list_tasks_for_connection(pool, &conn.id).await?;
    let enabled: Vec<_> = tasks.into_iter().filter(|t| t.enabled).collect();

    send_sync_tasks(conn, &enabled, write).await?;

    wait_for_message_type(
        conn,
        read,
        write,
        "sync_ok",
        std::time::Duration::from_secs(15),
        Some(pool),
    )
    .await?;

    if conn.is_production() {
        let cleared = db::clear_dump_passwords_for_connection(pool, &conn.id).await?;
        if cleared > 0 {
            tracing::info!(
                "[{}] Production mode: cleared {cleared} local DB password(s)",
                conn.slug
            );
        }
    }

    Ok(())
}
