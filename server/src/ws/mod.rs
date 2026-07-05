//! WebSocket handler: auth, sync_tasks, run_task.

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use futures::{SinkExt, StreamExt};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::db;
use crate::run_queue::RunJob;
use crate::state::{AppState, DeviceAuthOutcome};
use crate::task_registry::TaskDefinition;
use crate::types::{self, ClientMessage, ServerMessage};

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    debug!("WebSocket connection attempt");
    ws.on_upgrade(move |socket| handle_connection(socket, state))
}

async fn handle_connection(socket: WebSocket, state: AppState) {
    let connection_id = Uuid::new_v4().to_string();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(100);
    let (write_socket, mut read_socket) = socket.split();
    let write_arc = Arc::new(Mutex::new(write_socket));

    let mut stop_rx = {
        let mut stop_guard = state.ws_stop.lock().await;
        if let Some(tx) = stop_guard.take() {
            let _ = tx.send(());
            info!("Closing previous WebSocket session for reconnect");
        }
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
        *stop_guard = Some(stop_tx);
        stop_rx
    };

    *state.is_connected.lock().await = true;

    let auth_result = authenticate(&mut read_socket, &state).await;

    match auth_result {
        Ok(outcome) => {
            let auth_label = match outcome {
                DeviceAuthOutcome::Registered => "device_id registered",
                DeviceAuthOutcome::Verified => "device_id verified",
            };
            info!("Client connected — {auth_label}");

            let pong = ServerMessage::Pong {
                status: "authenticated".to_string(),
            };
            if write_arc
                .lock()
                .await
                .send(Message::Text(pong.to_json().to_string()))
                .await
                .is_err()
            {
                return;
            }

            {
                let mut channels = state.client_channels.lock().await;
                channels.insert(connection_id.clone(), tx);
            }

            let write_for_send = Arc::clone(&write_arc);
            let send_task = tokio::spawn(async move {
                while let Some(msg) = rx.recv().await {
                    if write_for_send
                        .lock()
                        .await
                        .send(Message::Text(msg))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            });

            let mut last_pong = std::time::Instant::now();
            let ping_interval = tokio::time::Duration::from_secs(15);
            let pong_timeout = tokio::time::Duration::from_secs(90);

            loop {
                tokio::select! {
                    msg = read_socket.next() => {
                        match msg {
                            Some(Ok(Message::Text(text))) => {
                                last_pong = std::time::Instant::now();
                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                                    if json.get("type").and_then(|v| v.as_str()) == Some("pong") {
                                        continue;
                                    }
                                }

                                match serde_json::from_str::<ClientMessage>(&text) {
                                    Ok(client_msg) => {
                                        handle_message(&state, client_msg, &connection_id).await;
                                    }
                                    Err(e) => warn!("Invalid client message: {}", e),
                                }
                            }
                            Some(Ok(Message::Close(_))) | None => break,
                            Some(Err(e)) => {
                                warn!("WebSocket error: {}", e);
                                break;
                            }
                            _ => {}
                        }
                    }
                    _ = &mut stop_rx => {
                        info!("WebSocket session replaced by reconnect");
                        break;
                    }
                    _ = tokio::time::sleep(ping_interval) => {
                        if last_pong.elapsed() > pong_timeout {
                            warn!("Client heartbeat timeout, closing connection");
                            break;
                        }
                        let ping = Message::Text(r#"{"type":"ping"}"#.into());
                        if write_arc.lock().await.send(ping).await.is_err() {
                            break;
                        }
                    }
                }
            }

            send_task.abort();
            state.client_channels.lock().await.remove(&connection_id);
            cleanup_connection(&state, &connection_id).await;
            *state.is_connected.lock().await = false;
            info!("Client disconnected (connection: {})", connection_id);
        }
        Err(e) => {
            warn!("Connection denied — device_id auth failed: {e}");
            let error_msg = ServerMessage::Error {
                message: "Device not authorized — device_id rejected".to_string(),
                def_id: None,
            };
            let mut write = write_arc.lock().await;
            let _ = write
                .send(Message::Text(error_msg.to_json().to_string()))
                .await;
            let _ = write.send(Message::Close(None)).await;
            drop(write);
            *state.is_connected.lock().await = false;
        }
    }
}

async fn cleanup_connection(state: &AppState, connection_id: &str) {
    state.task_registry.remove_connection(connection_id).await;

    let cancelled = state.run_queue.cancel_for_connection(connection_id).await;
    if !cancelled.is_empty() {
        debug!(
            "Cancelled {} pending run(s) for disconnected client",
            cancelled.len()
        );
    }

    state.busy_connections.lock().await.remove(connection_id);
}

async fn handle_message(state: &AppState, msg: ClientMessage, connection_id: &str) {
    match msg {
        ClientMessage::Auth { .. } => {
            warn!("Received auth after authentication (connection: {})", connection_id);
        }
        ClientMessage::SyncTasks { tasks } => {
            let mut definitions = Vec::with_capacity(tasks.len());

            for item in tasks {
                let task_type = crate::task_types::normalize(&item.task_type).to_string();
                let mut data = item.data;

                if let Err(e) = types::normalize_task_data(&task_type, &mut data, &state.config) {
                    send_to_client(
                        state,
                        connection_id,
                        &ServerMessage::Error {
                            message: format!("Invalid task {}: {}", item.def_id, e),
                            def_id: Some(item.def_id.clone()),
                        },
                    )
                    .await;
                    return;
                }

                definitions.push(TaskDefinition {
                    def_id: item.def_id,
                    task_name: item.task_name,
                    task_type,
                    data,
                });
            }

            let count = definitions.len();
            let merged = match db::apply_sync(&state.db, definitions).await {
                Ok(tasks) => tasks,
                Err(e) => {
                    send_to_client(
                        state,
                        connection_id,
                        &ServerMessage::Error {
                            message: format!("Failed to persist tasks: {e}"),
                            def_id: None,
                        },
                    )
                    .await;
                    return;
                }
            };

            for def in &merged {
                if types::task_encrypt_enabled(&def.task_type, &def.data, &state.config)
                    && !state.config.has_encrypt_key()
                {
                    warn!(
                        "Task '{}' ({}): Encrypt mode on but encrypt_password is not set — runs will be skipped",
                        def.task_name, def.def_id
                    );
                }
            }

            state
                .task_registry
                .replace_connection(connection_id, merged)
                .await;

            if state.config.is_debug() {
                debug!("Synced {} task definition(s) for {}", count, connection_id);
            } else {
                info!("Synced {} task(s) to backup.db", count);
            }

            send_to_client(
                state,
                connection_id,
                &ServerMessage::SyncOk { count },
            )
            .await;
        }
        ClientMessage::RunTask { def_id } => {
            let Some(def) = state.task_registry.get(connection_id, &def_id).await else {
                send_to_client(
                    state,
                    connection_id,
                    &ServerMessage::Error {
                        message: format!("Unknown task definition: {}", def_id),
                        def_id: Some(def_id.clone()),
                    },
                )
                .await;
                return;
            };

            if state.config.is_debug() {
                debug!(
                    "Run: {} | {} | def {}",
                    def.task_name, def.task_type, def_id
                );
            } else {
                info!("{} | {} | queued", def.task_name, def.task_type);
            }

            let run_id = types::generate_run_id();
            state
                .run_queue
                .enqueue(RunJob {
                    run_id: run_id.clone(),
                    def_id: def_id.clone(),
                    connection_id: connection_id.to_string(),
                })
                .await;

            let queue_position = state.run_queue.len().await as i32;
            let response = ServerMessage::TaskQueued {
                task_id: run_id,
                def_id: def_id.clone(),
                queue_position,
            };
            send_to_client(state, connection_id, &response).await;
        }
        ClientMessage::CheckDownload { filename } => {
            if let Ok(true) = db::commit_dir_sync_download(&state.db, &filename).await {
                info!("dir_sync state updated after download: {}", filename);
            }
            crate::temp_files::delete_temp_file_best_effort(&state.config.files_dir, &filename)
                .await;
            if !state.config.is_debug() {
                info!("Removed temp file: {}", filename);
            } else {
                debug!("Removed temp file: {}", filename);
            }
        }
        ClientMessage::ResetDirSync { def_id } => {
            let Some(def) = state.task_registry.get(connection_id, &def_id).await else {
                send_to_client(
                    state,
                    connection_id,
                    &ServerMessage::Error {
                        message: format!("Unknown task definition: {}", def_id),
                        def_id: Some(def_id.clone()),
                    },
                )
                .await;
                return;
            };

            if crate::task_types::normalize(&def.task_type) != "dir_sync" {
                send_to_client(
                    state,
                    connection_id,
                    &ServerMessage::Error {
                        message: format!("Task {} is not dir_sync", def.task_name),
                        def_id: Some(def_id.clone()),
                    },
                )
                .await;
                return;
            }

            if let Err(e) = db::reset_dir_sync_state(&state.db, &def_id).await {
                send_to_client(
                    state,
                    connection_id,
                    &ServerMessage::Error {
                        message: format!("Failed to reset dir_sync: {e}"),
                        def_id: Some(def_id.clone()),
                    },
                )
                .await;
                return;
            }

            info!("dir_sync reset: {} ({})", def.task_name, def_id);
            send_to_client(
                state,
                connection_id,
                &ServerMessage::DirSyncResetOk { def_id },
            )
            .await;
        }
        ClientMessage::Pong => {}
    }
}

async fn send_to_client(state: &AppState, connection_id: &str, msg: &ServerMessage) {
    let channels = state.client_channels.lock().await;
    if let Some(tx) = channels.get(connection_id) {
        let _ = tx.send(msg.to_json().to_string()).await;
    }
}

async fn authenticate<S>(socket: &mut S, state: &AppState) -> anyhow::Result<DeviceAuthOutcome>
where
    S: futures::Stream<Item = Result<Message, axum::Error>> + Unpin,
{
    let msg = tokio::time::timeout(std::time::Duration::from_secs(10), socket.next()).await;

    match msg {
        Ok(Some(Ok(Message::Text(text)))) => {
            let json: serde_json::Value = serde_json::from_str(&text)?;
            if json.get("type").and_then(|v| v.as_str()) != Some("auth") {
                anyhow::bail!("First message must be auth");
            }
            let device_id = json
                .get("device_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing device_id field"))?;
            state.verify_or_register_device(device_id).await
        }
        Ok(Some(Ok(_))) => anyhow::bail!("Expected text message for auth"),
        Ok(Some(Err(e))) => anyhow::bail!("WebSocket error during auth: {}", e),
        Ok(None) => anyhow::bail!("Connection closed during auth"),
        Err(_) => anyhow::bail!("Auth timeout (10 seconds)"),
    }
}
