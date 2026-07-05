//! Dispatch backup task by type to backup/* handlers.

use anyhow::Result;
use sqlx::SqlitePool;

use crate::backup::{db_dump, dir_sync, file_archive, shell, TaskOutcome, TaskResult};
use crate::config::Config;
use crate::task_types;

pub async fn execute_task(
    task_id: &str,
    task_type: &str,
    data: &serde_json::Value,
    task_name: &str,
    def_id: &str,
    config: &Config,
    db: &SqlitePool,
) -> Result<TaskOutcome> {
    let task_type = task_types::normalize(task_type);

    match task_type {
        "file_archive" | "files_archive" => {
            execute_files_archive(task_id, data, task_name)
                .await
                .map(TaskOutcome::File)
        }
        "dir_sync" => dir_sync::execute(def_id, task_name, data, db).await,
        "sqlite_dump" => db_dump::dump_sqlite(task_id, task_name, "sqlite_dump", data)
            .await
            .map(TaskOutcome::File),
        "mysql_dump" | "mariadb_dump" => {
            db_dump::dump_and_archive(
                task_id,
                task_name,
                "mysql_dump",
                data,
                &config.mysqldump_path,
                &config.pg_dump_path,
            )
            .await
            .map(TaskOutcome::File)
        }
        "postgresql_dump" => {
            db_dump::dump_and_archive(
                task_id,
                task_name,
                "postgresql_dump",
                data,
                &config.mysqldump_path,
                &config.pg_dump_path,
            )
            .await
            .map(TaskOutcome::File)
        }
        "shell" => execute_shell(task_id, data, task_name)
            .await
            .map(TaskOutcome::File),
        _ => Err(anyhow::anyhow!("Unknown task type: {}", task_type)),
    }
}

async fn execute_files_archive(
    _task_id: &str,
    task_data: &serde_json::Value,
    task_name: &str,
) -> Result<TaskResult> {
    let source_path = task_data
        .get("source_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing source_path"))?;

    let ignore_patterns = task_data
        .get("ignore")
        .and_then(|v| v.as_array())
        .map_or_else(Vec::new, |v| v.to_vec())
        .iter()
        .filter_map(|v| v.as_str())
        .map(|s| s.to_string())
        .collect();

    let files_dir = task_data
        .get("files_dir")
        .and_then(|v| v.as_str())
        .unwrap_or("./data/temp");

    let task = file_archive::FileArchiveTask {
        source_path: source_path.to_string(),
        ignore_patterns,
        files_dir: files_dir.to_string(),
    };

    task.execute(task_name, "files_archive").await
}

async fn execute_shell(
    _task_id: &str,
    task_data: &serde_json::Value,
    task_name: &str,
) -> Result<TaskResult> {
    let script_name = task_data
        .get("script_name")
        .or_else(|| task_data.get("script"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing script_name"))?;

    let script_args = task_data
        .get("script_args")
        .or_else(|| task_data.get("args"))
        .and_then(|v| v.as_array())
        .map_or_else(Vec::new, |v| v.to_vec())
        .iter()
        .filter_map(|v| v.as_str())
        .map(|s| s.to_string())
        .collect();

    let timeout_secs = task_data
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(300);

    let scripts_dir = task_data
        .get("scripts_dir")
        .and_then(|v| v.as_str())
        .unwrap_or("./data/scripts");

    let files_dir = task_data
        .get("files_dir")
        .and_then(|v| v.as_str())
        .unwrap_or("./data/temp");

    let task = shell::ShellTask {
        task_name: task_name.to_string(),
        script_name: script_name.to_string(),
        script_args,
        timeout_secs,
        scripts_dir: scripts_dir.to_string(),
        files_dir: files_dir.to_string(),
    };

    task.execute().await
}
