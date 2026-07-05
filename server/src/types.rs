//! WebSocket message types and task data validation.

use anyhow::Result;
use rand::Rng;
use serde::{Deserialize, Serialize};

use crate::config::Config;

#[derive(Debug, Deserialize)]
pub struct SyncTaskItem {
    pub def_id: String,
    pub task_name: String,
    pub task_type: String,
    pub data: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    #[serde(rename = "auth")]
    Auth {
        #[allow(dead_code)]
        device_id: String,
    },
    #[serde(rename = "sync_tasks")]
    SyncTasks { tasks: Vec<SyncTaskItem> },
    #[serde(rename = "run_task")]
    RunTask { def_id: String },
    #[serde(rename = "check_download")]
    CheckDownload { filename: String },
    #[serde(rename = "reset_dir_sync")]
    ResetDirSync { def_id: String },
    #[serde(rename = "pong")]
    Pong,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    #[serde(rename = "pong")]
    Pong { status: String },
    #[serde(rename = "sync_ok")]
    SyncOk { count: usize },
    #[serde(rename = "dir_sync_reset_ok")]
    DirSyncResetOk { def_id: String },
    #[serde(rename = "task_queued")]
    TaskQueued {
        task_id: String,
        def_id: String,
        queue_position: i32,
    },
    #[serde(rename = "task_completed")]
    TaskCompleted {
        task_id: String,
        def_id: String,
        status: String,
        download_url: String,
        file_size_bytes: i64,
        file_hash: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    #[serde(rename = "task_failed")]
    TaskFailed {
        task_id: String,
        def_id: String,
        error: String,
    },
    #[serde(rename = "error")]
    Error {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        def_id: Option<String>,
    },
}

impl ServerMessage {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

pub fn normalize_task_data(
    task_type: &str,
    data: &mut serde_json::Value,
    config: &Config,
) -> Result<()> {
    let obj = data
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("Task data must be a JSON object"))?;

    obj.entry("files_dir")
        .or_insert_with(|| {
            serde_json::Value::from(
                config
                    .files_dir_abs()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| config.files_dir.clone()),
            )
        });
    obj.entry("scripts_dir")
        .or_insert_with(|| {
            serde_json::Value::from(
                config
                    .scripts_dir_abs()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| config.scripts_dir.clone()),
            )
        });

    match task_type {
        "shell" => normalize_shell_fields(obj),
        "postgresql_dump" => {
            obj.entry("provider")
                .or_insert(serde_json::Value::from("postgresql"));
        }
        "mysql_dump" | "mariadb_dump" => {
            obj.entry("provider")
                .or_insert(serde_json::Value::from("mysql"));
        }
        "file_archive" | "files_archive" | "dir_sync" => {}
        _ => {}
    }

    if task_type == "dir_sync" {
        obj.insert("encrypt".to_string(), serde_json::Value::Bool(false));
        obj.entry("max_batch_mb")
            .or_insert(serde_json::Value::from(200_u64));
    } else {
        obj.entry("encrypt")
            .or_insert(serde_json::Value::Bool(false));
    }

    Ok(())
}

/// Per-task encrypt flag from synced `data_json`. Legacy tasks without `encrypt` fall back to
/// global `encrypt_backups` in server config.
pub fn task_encrypt_enabled(
    task_type: &str,
    data: &serde_json::Value,
    config: &Config,
) -> bool {
    if crate::task_types::normalize(task_type) == "dir_sync" {
        return false;
    }
    match data.get("encrypt") {
        Some(v) => v.as_bool().unwrap_or(false),
        None => config.encrypt_backups,
    }
}

fn normalize_shell_fields(data: &mut serde_json::Map<String, serde_json::Value>) {
    if !data.contains_key("script_name") {
        if let Some(script) = data.get("script").and_then(|v| v.as_str()) {
            data.insert("script_name".to_string(), serde_json::Value::from(script));
        }
    }
    if !data.contains_key("script_args") {
        if let Some(args) = data.get("args") {
            data.insert("script_args".to_string(), args.clone());
        }
    }
}

pub fn generate_run_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();

    let suffix: String = (0..8)
        .map(|_| {
            let hex = "0123456789abcdef";
            let idx = rand::thread_rng().gen_range(0..16);
            hex.chars().nth(idx).unwrap_or('0')
        })
        .collect();

    format!("run_{}_{suffix}", timestamp % 1_000_000_000)
}
