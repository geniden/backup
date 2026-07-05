//! Persistent task storage on server (`backup.db`).

use anyhow::Context;
use chrono::Utc;
use sqlx::{sqlite::SqlitePoolOptions, Row, SqlitePool};
use tracing::info;

use crate::paths;
use crate::task_registry::TaskDefinition;

pub async fn init_db() -> anyhow::Result<SqlitePool> {
    let db_path = paths::db_path()?;
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
    }
    if !db_path.exists() {
        std::fs::File::create(&db_path)
            .with_context(|| format!("Failed to create {}", db_path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&db_path, std::fs::Permissions::from_mode(0o600))?;
        }
    }

    let url = paths::sqlite_url(&db_path);
    let pool = SqlitePoolOptions::new()
        .max_connections(3)
        .connect(&url)
        .await
        .context("Failed to open backup.db")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS tasks (
            def_id TEXT PRIMARY KEY,
            task_name TEXT NOT NULL,
            task_type TEXT NOT NULL,
            data_json TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query("PRAGMA journal_mode=WAL")
        .execute(&pool)
        .await?;
    sqlx::query("PRAGMA synchronous=NORMAL")
        .execute(&pool)
        .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS task_sync_state (
            def_id TEXT PRIMARY KEY,
            sync_last INTEGER NOT NULL DEFAULT 0,
            sync_last_path TEXT NOT NULL DEFAULT '',
            updated_at TEXT NOT NULL
        );
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS dir_sync_pending (
            filename TEXT PRIMARY KEY,
            def_id TEXT NOT NULL,
            sync_mtime INTEGER NOT NULL,
            sync_last_path TEXT NOT NULL DEFAULT ''
        );
        "#,
    )
    .execute(&pool)
    .await?;

    migrate_dir_sync_cursor_columns(&pool).await?;

    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM tasks")
        .fetch_one(&pool)
        .await?;
    if count.0 > 0 {
        info!("Loaded {} task definition(s) from backup.db", count.0);
    }

    Ok(pool)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DirSyncCursor {
    pub sync_last: i64,
    pub sync_last_path: String,
}

async fn migrate_dir_sync_cursor_columns(pool: &SqlitePool) -> anyhow::Result<()> {
    migrate_add_column_if_missing(pool, "task_sync_state", "sync_last_path", "TEXT NOT NULL DEFAULT ''").await?;
    migrate_add_column_if_missing(pool, "dir_sync_pending", "sync_last_path", "TEXT NOT NULL DEFAULT ''").await?;
    Ok(())
}

async fn migrate_add_column_if_missing(
    pool: &SqlitePool,
    table: &str,
    column: &str,
    definition: &str,
) -> anyhow::Result<()> {
    let rows = sqlx::query(&format!("PRAGMA table_info({table})"))
        .fetch_all(pool)
        .await?;
    for row in &rows {
        let name: String = row.try_get("name")?;
        if name == column {
            return Ok(());
        }
    }
    let sql = format!("ALTER TABLE {table} ADD COLUMN {column} {definition}");
    sqlx::query(&sql).execute(pool).await?;
    Ok(())
}

pub fn is_dump_task(task_type: &str) -> bool {
    matches!(
        task_type,
        "mysql_dump" | "mariadb_dump" | "postgresql_dump"
    )
}

fn db_pass_from_data(data: &serde_json::Value) -> Option<&str> {
    data.get("db_pass")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
}

/// Merge incoming sync with stored task: empty client password does not overwrite server.
pub fn merge_task(
    incoming: &TaskDefinition,
    stored_data: Option<&serde_json::Value>,
) -> TaskDefinition {
    let mut data = incoming.data.clone();

    if is_dump_task(&incoming.task_type) && db_pass_from_data(&data).is_none() {
        if let Some(prev) = stored_data {
            if let Some(pass) = db_pass_from_data(prev) {
                if let Some(obj) = data.as_object_mut() {
                    obj.insert(
                        "db_pass".to_string(),
                        serde_json::Value::String(pass.to_string()),
                    );
                }
            }
        }
    }

    TaskDefinition {
        def_id: incoming.def_id.clone(),
        task_name: incoming.task_name.clone(),
        task_type: incoming.task_type.clone(),
        data,
    }
}

pub async fn get_task_data(
    pool: &SqlitePool,
    def_id: &str,
) -> anyhow::Result<Option<serde_json::Value>> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT data_json FROM tasks WHERE def_id = ?")
            .bind(def_id)
            .fetch_optional(pool)
            .await?;

    Ok(row.map(|(data_json,)| {
        serde_json::from_str(&data_json).unwrap_or(serde_json::Value::Object(Default::default()))
    }))
}

pub async fn upsert_task(pool: &SqlitePool, task: &TaskDefinition) -> anyhow::Result<()> {
    let data_json = serde_json::to_string(&task.data)?;
    let updated_at = Utc::now().to_rfc3339();
    sqlx::query(
        r#"
        INSERT INTO tasks (def_id, task_name, task_type, data_json, updated_at)
        VALUES (?, ?, ?, ?, ?)
        ON CONFLICT(def_id) DO UPDATE SET
            task_name = excluded.task_name,
            task_type = excluded.task_type,
            data_json = excluded.data_json,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(&task.def_id)
    .bind(&task.task_name)
    .bind(&task.task_type)
    .bind(&data_json)
    .bind(&updated_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_tasks_not_in(pool: &SqlitePool, def_ids: &[String]) -> anyhow::Result<()> {
    if def_ids.is_empty() {
        sqlx::query("DELETE FROM tasks")
            .execute(pool)
            .await?;
        return Ok(());
    }

    let placeholders = def_ids.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
    let sql = format!("DELETE FROM tasks WHERE def_id NOT IN ({placeholders})");
    let mut query = sqlx::query(&sql);
    for id in def_ids {
        query = query.bind(id);
    }
    query.execute(pool).await?;
    Ok(())
}

pub async fn apply_sync(
    pool: &SqlitePool,
    incoming: Vec<TaskDefinition>,
) -> anyhow::Result<Vec<TaskDefinition>> {
    let mut merged = Vec::with_capacity(incoming.len());

    for item in incoming {
        let stored_data = get_task_data(pool, &item.def_id).await?;
        let task = merge_task(&item, stored_data.as_ref());
        upsert_task(pool, &task).await?;
        merged.push(task);
    }

    let ids: Vec<String> = merged.iter().map(|t| t.def_id.clone()).collect();
    delete_tasks_not_in(pool, &ids).await?;

    Ok(merged)
}

pub async fn get_dir_sync_cursor(pool: &SqlitePool, def_id: &str) -> anyhow::Result<DirSyncCursor> {
    let row: Option<(i64, String)> = sqlx::query_as(
        "SELECT sync_last, sync_last_path FROM task_sync_state WHERE def_id = ?",
    )
    .bind(def_id)
    .fetch_optional(pool)
    .await?;
    Ok(row
        .map(|(sync_last, sync_last_path)| DirSyncCursor {
            sync_last,
            sync_last_path,
        })
        .unwrap_or_default())
}

pub async fn set_dir_sync_cursor(
    pool: &SqlitePool,
    def_id: &str,
    cursor: &DirSyncCursor,
) -> anyhow::Result<()> {
    let updated_at = Utc::now().to_rfc3339();
    sqlx::query(
        r#"
        INSERT INTO task_sync_state (def_id, sync_last, sync_last_path, updated_at)
        VALUES (?, ?, ?, ?)
        ON CONFLICT(def_id) DO UPDATE SET
            sync_last = excluded.sync_last,
            sync_last_path = excluded.sync_last_path,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(def_id)
    .bind(cursor.sync_last)
    .bind(&cursor.sync_last_path)
    .bind(updated_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn insert_dir_sync_pending(
    pool: &SqlitePool,
    filename: &str,
    def_id: &str,
    sync_mtime: i64,
    sync_last_path: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT OR REPLACE INTO dir_sync_pending (filename, def_id, sync_mtime, sync_last_path) VALUES (?, ?, ?, ?)",
    )
    .bind(filename)
    .bind(def_id)
    .bind(sync_mtime)
    .bind(sync_last_path)
    .execute(pool)
    .await?;
    Ok(())
}

/// After client `check_download`: commit cursor and drop pending row.
pub async fn commit_dir_sync_download(pool: &SqlitePool, filename: &str) -> anyhow::Result<bool> {
    let row: Option<(String, i64, String)> = sqlx::query_as(
        "SELECT def_id, sync_mtime, sync_last_path FROM dir_sync_pending WHERE filename = ?",
    )
    .bind(filename)
    .fetch_optional(pool)
    .await?;

    let Some((def_id, sync_mtime, sync_last_path)) = row else {
        return Ok(false);
    };

    set_dir_sync_cursor(
        pool,
        &def_id,
        &DirSyncCursor {
            sync_last: sync_mtime,
            sync_last_path,
        },
    )
    .await?;
    sqlx::query("DELETE FROM dir_sync_pending WHERE filename = ?")
        .bind(filename)
        .execute(pool)
        .await?;
    Ok(true)
}

/// Clear incremental sync cursor and pending batches for one task definition.
pub async fn reset_dir_sync_state(pool: &SqlitePool, def_id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM task_sync_state WHERE def_id = ?")
        .bind(def_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM dir_sync_pending WHERE def_id = ?")
        .bind(def_id)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_keeps_server_password_when_client_empty() {
        let incoming = TaskDefinition {
            def_id: "t1".into(),
            task_name: "db".into(),
            task_type: "mysql_dump".into(),
            data: json!({"db_name":"x","db_user":"u","db_pass":""}),
        };
        let stored = json!({"db_name":"x","db_user":"u","db_pass":"secret"});
        let merged = merge_task(&incoming, Some(&stored));
        assert_eq!(merged.data["db_pass"], "secret");
    }

    #[test]
    fn merge_updates_when_client_sends_password() {
        let incoming = TaskDefinition {
            def_id: "t1".into(),
            task_name: "db".into(),
            task_type: "postgresql_dump".into(),
            data: json!({"db_pass":"new"}),
        };
        let stored = json!({"db_pass":"old"});
        let merged = merge_task(&incoming, Some(&stored));
        assert_eq!(merged.data["db_pass"], "new");
    }
}
