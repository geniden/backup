//! Background worker: execute queued runs, notify client.

use tracing::{debug, info, warn};

use crate::backup::TaskOutcome;
use crate::executor;
use crate::run_queue::RunJob;
use crate::state::AppState;
use crate::task_types;
use crate::types::ServerMessage;

pub async fn run(state: AppState, shutdown_rx: &mut tokio::sync::broadcast::Receiver<()>) {
    debug!("Task worker started");

    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => break,
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(2)) => {
                if let Err(e) = poll_once(&state).await {
                    warn!("Worker error: {}", e);
                }
            }
        }
    }
}

async fn poll_once(state: &AppState) -> anyhow::Result<()> {
    let busy = state.busy_connections.lock().await.clone();
    let Some(job) = state.run_queue.pop_ready(&busy).await else {
        return Ok(());
    };

    process_job(state, job).await;
    Ok(())
}

async fn process_job(state: &AppState, job: RunJob) {
    let RunJob {
        run_id,
        def_id,
        connection_id,
    } = job;

    let Some(def) = state
        .task_registry
        .get(&connection_id, &def_id)
        .await
    else {
        warn!(
            "Run {} skipped: definition {} not in memory (connection {})",
            run_id, def_id, connection_id
        );
        notify_client(
            state,
            &connection_id,
            &ServerMessage::TaskFailed {
                task_id: run_id.clone(),
                def_id: def_id.clone(),
                error: "Task definition not available (client disconnected?)".to_string(),
            },
        )
        .await;
        return;
    };

    let task_name = def.task_name.as_str();
    let task_type = task_types::normalize(&def.task_type);
    let task_data = &def.data;

    state
        .busy_connections
        .lock()
        .await
        .insert(connection_id.clone());

    let result = executor::execute_task(
        &run_id,
        task_type,
        task_data,
        task_name,
        &def_id,
        &state.config,
        &state.db,
    )
    .await;

    match result {
        Ok(TaskOutcome::NoChanges { detail }) => {
            let log_detail = detail.unwrap_or("no_changes");
            info!("{} | {} | no new files to sync ({log_detail})", task_name, task_type);
            notify_client(
                state,
                &connection_id,
                &ServerMessage::TaskCompleted {
                    task_id: run_id.clone(),
                    def_id: def_id.clone(),
                    status: "no_changes".to_string(),
                    download_url: String::new(),
                    file_size_bytes: 0,
                    file_hash: String::new(),
                    detail: detail.map(str::to_string),
                },
            )
            .await;
        }
        Ok(TaskOutcome::File(result)) => {
            let files_dir = match state.config.files_dir_abs() {
                Ok(p) => p,
                Err(e) => {
                    warn!("{} | {} | encrypt path error: {}", task_name, task_type, e);
                    notify_client(
                        state,
                        &connection_id,
                        &ServerMessage::TaskFailed {
                            task_id: run_id.clone(),
                            def_id: def_id.clone(),
                            error: e.to_string(),
                        },
                    )
                    .await;
                    state.busy_connections.lock().await.remove(&connection_id);
                    return;
                }
            };

            let want_encrypt =
                crate::types::task_encrypt_enabled(task_type, task_data, &state.config);

            let result = if !want_encrypt {
                result
            } else if !state.config.has_encrypt_key() {
                let error = format!(
                    "Task '{task_name}' ({def_id}): Encrypt mode on but encrypt_password is not set on backup-server"
                );
                warn!("{} | {} | skipped: {}", task_name, task_type, error);
                notify_client(
                    state,
                    &connection_id,
                    &ServerMessage::TaskFailed {
                        task_id: run_id.clone(),
                        def_id: def_id.clone(),
                        error,
                    },
                )
                .await;
                state.busy_connections.lock().await.remove(&connection_id);
                return;
            } else {
                match crate::backup::encrypt::maybe_encrypt_backup_file(
                    &state.config,
                    &files_dir,
                    result,
                )
                .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        warn!("{} | {} | encrypt failed: {}", task_name, task_type, e);
                        notify_client(
                            state,
                            &connection_id,
                            &ServerMessage::TaskFailed {
                                task_id: run_id.clone(),
                                def_id: def_id.clone(),
                                error: format!("Encryption failed: {e}"),
                            },
                        )
                        .await;
                        state.busy_connections.lock().await.remove(&connection_id);
                        return;
                    }
                }
            };

            info!(
                "{} | {} | completed {} ({})",
                task_name,
                task_type,
                result.filename,
                crate::utils::format_bytes(result.size_bytes.max(0) as u64)
            );

            let response = ServerMessage::TaskCompleted {
                task_id: run_id.clone(),
                def_id: def_id.clone(),
                status: "success".to_string(),
                download_url: format!(
                    "{}://{}:{}/download/{}",
                    state.config.download_scheme(),
                    state.config.public_ip,
                    state.config.server_port,
                    result.filename
                ),
                file_size_bytes: result.size_bytes,
                file_hash: result.file_hash,
                detail: None,
            };

            notify_client(state, &connection_id, &response).await;
        }
        Err(e) => {
            warn!("{} | {} | failed: {}", task_name, task_type, e);
            notify_client(
                state,
                &connection_id,
                &ServerMessage::TaskFailed {
                    task_id: run_id.clone(),
                    def_id: def_id.clone(),
                    error: e.to_string(),
                },
            )
            .await;
        }
    }

    state.busy_connections.lock().await.remove(&connection_id);
}

async fn notify_client(state: &AppState, connection_id: &str, msg: &ServerMessage) {
    let Ok(json) = serde_json::to_string(msg) else {
        return;
    };
    let channels = state.client_channels.lock().await;
    if let Some(tx) = channels.get(connection_id) {
        let _ = tx.send(json).await;
    }
}
