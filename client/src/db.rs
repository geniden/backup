//! SQLite schema, migrations, WAL, task_runs history.

use chrono::Utc;
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::path::{Path, PathBuf};

use crate::models::connection::Connection;
use crate::paths;

pub async fn init_db() -> anyhow::Result<SqlitePool> {
    paths::ensure_layout()?;

    let db_path = paths::db_path();
    if !db_path.exists() {
        std::fs::write(&db_path, "")?;
    }

    let pool = connect_pool(&db_path, false).await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS connections (
            id TEXT PRIMARY KEY,
            slug TEXT NOT NULL UNIQUE,
            url TEXT NOT NULL,
            api_key TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            secrets_mode TEXT NOT NULL DEFAULT 'test'
        );
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS tasks (
            id TEXT PRIMARY KEY,
            connection_id TEXT NOT NULL,
            task_name TEXT NOT NULL,
            task_type TEXT NOT NULL,
            data_json TEXT NOT NULL,
            schedule TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            last_run TEXT,
            created_at TEXT NOT NULL,
            FOREIGN KEY (connection_id) REFERENCES connections(id)
        );
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS task_runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id TEXT NOT NULL,
            run_at TEXT NOT NULL,
            status TEXT NOT NULL,
            file_size_bytes INTEGER,
            file_path TEXT,
            error TEXT,
            anomaly_flags TEXT,
            server_run_id TEXT,
            FOREIGN KEY (task_id) REFERENCES tasks(id)
        );
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_task_runs_task_id_run_at ON task_runs(task_id, run_at DESC)",
    )
    .execute(&pool)
    .await?;

    migrate(&pool).await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS client_settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        "#,
    )
    .execute(&pool)
    .await?;

    let root = backups_root_path(&pool).await?;
    paths::ensure_backups_root_exists(&root)?;

    let _ = crate::ca::ensure_ca();
    Ok(pool)
}

const SETTING_BACKUPS_ROOT: &str = "backups_root";

pub async fn get_client_setting(
    pool: &SqlitePool,
    key: &str,
) -> anyhow::Result<Option<String>> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT value FROM client_settings WHERE key = ?")
            .bind(key)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(v,)| v))
}

pub async fn set_client_setting(
    pool: &SqlitePool,
    key: &str,
    value: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO client_settings (key, value) VALUES (?, ?)
        ON CONFLICT(key) DO UPDATE SET value = excluded.value
        "#,
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn clear_client_setting(pool: &SqlitePool, key: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM client_settings WHERE key = ?")
        .bind(key)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn backups_root_path(pool: &SqlitePool) -> anyhow::Result<PathBuf> {
    let custom = get_client_setting(pool, SETTING_BACKUPS_ROOT).await?;
    Ok(paths::resolve_backups_root(custom.as_deref()))
}

pub async fn backups_root_custom(pool: &SqlitePool) -> anyhow::Result<Option<String>> {
    Ok(get_client_setting(pool, SETTING_BACKUPS_ROOT).await?
        .filter(|s| !s.trim().is_empty()))
}

pub async fn set_backups_root_custom(pool: &SqlitePool, absolute_path: &str) -> anyhow::Result<()> {
    set_client_setting(pool, SETTING_BACKUPS_ROOT, absolute_path).await
}

pub async fn reset_backups_root_default(pool: &SqlitePool) -> anyhow::Result<()> {
    clear_client_setting(pool, SETTING_BACKUPS_ROOT).await
}

pub async fn open_readonly(db_path: Option<&Path>) -> anyhow::Result<SqlitePool> {
    let db_path = db_path.map(Path::to_path_buf).unwrap_or_else(paths::db_path);
    if !db_path.exists() {
        anyhow::bail!("Database not found: {}", db_path.display());
    }

    connect_pool(&db_path, true).await
}

async fn connect_pool(db_path: &Path, read_only: bool) -> anyhow::Result<SqlitePool> {
    let url = if read_only {
        format!("sqlite:{}?mode=ro", db_path.display())
    } else {
        format!("sqlite:{}", db_path.display())
    };

    let pool = SqlitePoolOptions::new()
        .max_connections(if read_only { 2 } else { 5 })
        .connect(&url)
        .await?;

    if !read_only {
        configure_wal(&pool).await?;
    }

    Ok(pool)
}

async fn configure_wal(pool: &SqlitePool) -> anyhow::Result<()> {
    let mode: (String,) = sqlx::query_as("PRAGMA journal_mode=WAL")
        .fetch_one(pool)
        .await?;
    if mode.0.to_lowercase() != "wal" {
        tracing::warn!("SQLite journal_mode is {}, expected WAL", mode.0);
    }
    sqlx::query("PRAGMA synchronous=NORMAL")
        .execute(pool)
        .await?;
    Ok(())
}

async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    let _ = sqlx::query("ALTER TABLE connections RENAME COLUMN name TO slug")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE tasks RENAME COLUMN session_id TO task_name")
        .execute(pool)
        .await;
    let _ = sqlx::query(
        "UPDATE tasks SET task_name = name WHERE (task_name IS NULL OR task_name = '') AND name IS NOT NULL",
    )
    .execute(pool)
    .await;
    let _ = sqlx::query("ALTER TABLE tasks DROP COLUMN name")
        .execute(pool)
        .await;

    let _ = sqlx::query(
        "ALTER TABLE connections ADD COLUMN tls_enabled INTEGER NOT NULL DEFAULT 0",
    )
    .execute(pool)
    .await;
    let _ = sqlx::query("ALTER TABLE connections ADD COLUMN cert_fingerprint TEXT")
        .execute(pool)
        .await;
    let _ = sqlx::query(
        "ALTER TABLE connections ADD COLUMN secrets_mode TEXT NOT NULL DEFAULT 'test'",
    )
    .execute(pool)
    .await;

    sqlx::query(
        "UPDATE connections SET url = REPLACE(url, 'ws://', 'wss://') WHERE tls_enabled = 1 AND url LIKE 'ws://%'",
    )
    .execute(pool)
    .await?;

    sqlx::query("UPDATE connections SET tls_enabled = 1 WHERE tls_enabled = 0")
        .execute(pool)
        .await?;
    sqlx::query("UPDATE connections SET url = REPLACE(url, 'ws://', 'wss://') WHERE url LIKE 'ws://%'")
        .execute(pool)
        .await?;

    let _ = sqlx::query(
        "ALTER TABLE connections ADD COLUMN retention_days INTEGER NOT NULL DEFAULT 0",
    )
    .execute(pool)
    .await;
    let _ = sqlx::query("ALTER TABLE connections ADD COLUMN retention_last_run TEXT")
        .execute(pool)
        .await;

    Ok(())
}

pub async fn add_connection(
    pool: &SqlitePool,
    slug: &str,
    url: &str,
    cert_fingerprint: Option<&str>,
) -> anyhow::Result<String> {
    let id = format!("conn_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO connections (id, slug, url, api_key, tls_enabled, cert_fingerprint, enabled, created_at) VALUES (?, ?, ?, '', 1, ?, 1, ?)",
    )
    .bind(&id)
    .bind(slug)
    .bind(url)
    .bind(cert_fingerprint)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn connection_url_exists(
    pool: &SqlitePool,
    url: &str,
    exclude_id: Option<&str>,
) -> anyhow::Result<bool> {
    let count = if let Some(id) = exclude_id {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM connections WHERE url = ? AND id != ?",
        )
        .bind(url)
        .bind(id)
        .fetch_one(pool)
        .await?
    } else {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM connections WHERE url = ?")
            .bind(url)
            .fetch_one(pool)
            .await?
    };
    Ok(count > 0)
}

pub async fn update_connection(
    pool: &SqlitePool,
    id: &str,
    slug: &str,
    url: &str,
    cert_fingerprint: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE connections SET slug = ?, url = ?, api_key = '', tls_enabled = 1, cert_fingerprint = ? WHERE id = ?",
    )
    .bind(slug)
    .bind(url)
    .bind(cert_fingerprint)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_connection_tls(
    pool: &SqlitePool,
    id: &str,
    url: &str,
    cert_fingerprint: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE connections SET url = ?, api_key = '', tls_enabled = 1, cert_fingerprint = ? WHERE id = ?",
    )
    .bind(url)
    .bind(cert_fingerprint)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

const CONNECTION_SELECT: &str = "SELECT id, slug, url, api_key, tls_enabled, cert_fingerprint, enabled, created_at, \
    COALESCE(secrets_mode, 'test') AS secrets_mode, COALESCE(retention_days, 0) AS retention_days, retention_last_run \
    FROM connections";

pub async fn get_connection(pool: &SqlitePool, id: &str) -> anyhow::Result<Option<Connection>> {
    sqlx::query_as::<_, Connection>(&format!("{CONNECTION_SELECT} WHERE id = ?"))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

pub async fn get_task_by_id(
    pool: &SqlitePool,
    id: &str,
) -> anyhow::Result<Option<crate::models::task::Task>> {
    sqlx::query_as(
        "SELECT id, connection_id, task_name, task_type, data_json, schedule, enabled, last_run, created_at FROM tasks WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

pub async fn list_connections(pool: &SqlitePool) -> Result<Vec<Connection>, sqlx::Error> {
    list_enabled_connections(pool).await
}

pub async fn list_all_connections(pool: &SqlitePool) -> Result<Vec<Connection>, sqlx::Error> {
    sqlx::query_as::<_, Connection>(&format!("{CONNECTION_SELECT} ORDER BY slug"))
    .fetch_all(pool)
    .await
}

pub async fn list_enabled_connections(pool: &SqlitePool) -> Result<Vec<Connection>, sqlx::Error> {
    sqlx::query_as::<_, Connection>(&format!(
        "{CONNECTION_SELECT} WHERE enabled = 1 ORDER BY slug"
    ))
    .fetch_all(pool)
    .await
}

pub async fn update_connection_enabled(
    pool: &SqlitePool,
    id: &str,
    enabled: bool,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE connections SET enabled = ? WHERE id = ?")
        .bind(enabled as i32)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_connection_secrets_mode(
    pool: &SqlitePool,
    id: &str,
    mode: &str,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE connections SET secrets_mode = ? WHERE id = ?")
        .bind(mode)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_connection_retention(
    pool: &SqlitePool,
    id: &str,
    retention_days: i32,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE connections SET retention_days = ? WHERE id = ?")
        .bind(retention_days)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn touch_retention_last_run(pool: &SqlitePool, id: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE connections SET retention_last_run = ? WHERE id = ?")
        .bind(Utc::now().to_rfc3339())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn clear_task_run_file_path(pool: &SqlitePool, run_id: i64) -> anyhow::Result<()> {
    sqlx::query("UPDATE task_runs SET file_path = NULL WHERE id = ?")
        .bind(run_id)
        .execute(pool)
        .await?;
    Ok(())
}

#[derive(Debug, sqlx::FromRow)]
pub struct RetentionRunRow {
    pub id: i64,
    pub task_id: String,
    pub task_name: String,
    pub task_type: String,
    pub run_at: String,
    pub file_path: String,
}

pub async fn list_retention_runs(
    pool: &SqlitePool,
    connection_id: &str,
) -> anyhow::Result<Vec<RetentionRunRow>> {
    sqlx::query_as::<_, RetentionRunRow>(
        r#"
        SELECT tr.id, tr.task_id, t.task_name, t.task_type, tr.run_at, tr.file_path
        FROM task_runs tr
        INNER JOIN tasks t ON t.id = tr.task_id
        WHERE t.connection_id = ?
          AND tr.status IN ('ok', 'warn')
          AND tr.file_path IS NOT NULL
          AND tr.file_path != ''
        ORDER BY tr.task_id ASC, tr.run_at DESC
        "#,
    )
    .bind(connection_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

fn is_dump_task(task_type: &str) -> bool {
    let t = crate::validation::normalize_task_type(task_type);
    matches!(t, "mysql_dump" | "mariadb_dump" | "postgresql_dump")
}

/// Remove `db_pass` from dump tasks on the client (production mode after sync).
pub async fn clear_dump_passwords_for_connection(
    pool: &SqlitePool,
    connection_id: &str,
) -> anyhow::Result<usize> {
    let tasks = list_tasks_for_connection(pool, connection_id).await?;
    let mut cleared = 0usize;

    for task in tasks {
        if !is_dump_task(&task.task_type) {
            continue;
        }
        let mut data: serde_json::Value = serde_json::from_str(&task.data_json)?;
        let Some(obj) = data.as_object_mut() else {
            continue;
        };
        let has_pass = obj
            .get("db_pass")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty());
        if !has_pass {
            continue;
        }
        obj.insert("db_pass".to_string(), serde_json::Value::String(String::new()));
        update_task(
            pool,
            &task.id,
            &task.task_name,
            &task.task_type,
            &data.to_string(),
            &task.schedule,
            task.enabled,
        )
        .await?;
        cleared += 1;
    }

    Ok(cleared)
}

pub async fn connection_has_enabled_tasks(pool: &SqlitePool, connection_id: &str) -> bool {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM tasks WHERE connection_id = ? AND enabled = 1",
    )
    .bind(connection_id)
    .fetch_one(pool)
    .await
    .unwrap_or(0)
        > 0
}

pub async fn delete_connection(pool: &SqlitePool, id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM tasks WHERE connection_id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM connections WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_last_run(pool: &SqlitePool, task_id: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE tasks SET last_run = ? WHERE id = ?")
        .bind(Utc::now().to_rfc3339())
        .bind(task_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn add_task(
    pool: &SqlitePool,
    id: &str,
    connection_id: &str,
    task_name: &str,
    task_type: &str,
    data_json: &str,
    schedule: &str,
    enabled: bool,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tasks (id, connection_id, task_name, task_type, data_json, schedule, enabled, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(connection_id)
    .bind(task_name)
    .bind(task_type)
    .bind(data_json)
    .bind(schedule)
    .bind(enabled as i32)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_task(
    pool: &SqlitePool,
    id: &str,
    task_name: &str,
    task_type: &str,
    data_json: &str,
    schedule: &str,
    enabled: bool,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE tasks SET task_name = ?, task_type = ?, data_json = ?, schedule = ?, enabled = ? WHERE id = ?",
    )
    .bind(task_name)
    .bind(task_type)
    .bind(data_json)
    .bind(schedule)
    .bind(enabled as i32)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_tasks(pool: &SqlitePool) -> anyhow::Result<Vec<crate::models::task::Task>> {
    sqlx::query_as(
        "SELECT id, connection_id, task_name, task_type, data_json, schedule, enabled, last_run, created_at FROM tasks",
    )
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub async fn list_tasks_for_connection(
    pool: &SqlitePool,
    connection_id: &str,
) -> anyhow::Result<Vec<crate::models::task::Task>> {
    sqlx::query_as(
        "SELECT id, connection_id, task_name, task_type, data_json, schedule, enabled, last_run, created_at FROM tasks WHERE connection_id = ?",
    )
    .bind(connection_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub async fn update_task_enabled(pool: &SqlitePool, task_id: &str, enabled: bool) -> anyhow::Result<()> {
    sqlx::query("UPDATE tasks SET enabled = ? WHERE id = ?")
        .bind(enabled as i32)
        .bind(task_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_task(pool: &SqlitePool, task_id: &str) -> anyhow::Result<()> {
    clear_task_run_history(pool, task_id).await?;
    sqlx::query("DELETE FROM tasks WHERE id = ?")
        .bind(task_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Remove run history and last_run timestamp (used by dir_sync reset).
pub async fn clear_task_run_history(pool: &SqlitePool, task_id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM task_runs WHERE task_id = ?")
        .bind(task_id)
        .execute(pool)
        .await?;
    sqlx::query("UPDATE tasks SET last_run = NULL WHERE id = ?")
        .bind(task_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Latest successful dir_sync download path for a task.
pub async fn get_latest_sync_file_path(
    pool: &SqlitePool,
    task_id: &str,
) -> anyhow::Result<Option<String>> {
    let row = sqlx::query_scalar::<_, Option<String>>(
        r#"
        SELECT file_path FROM task_runs
        WHERE task_id = ?
          AND status IN ('ok', 'warn')
          AND file_path IS NOT NULL
          AND file_path != ''
          AND file_size_bytes IS NOT NULL
          AND file_size_bytes > 0
        ORDER BY run_at DESC
        LIMIT 1
        "#,
    )
    .bind(task_id)
    .fetch_optional(pool)
    .await?
    .flatten();
    Ok(row)
}

pub async fn get_previous_ok_run_size(
    pool: &SqlitePool,
    task_id: &str,
) -> anyhow::Result<Option<i64>> {
    let size = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT file_size_bytes FROM task_runs WHERE task_id = ? AND status IN ('ok', 'warn') ORDER BY run_at DESC LIMIT 1",
    )
    .bind(task_id)
    .fetch_optional(pool)
    .await?
    .flatten();
    Ok(size)
}

pub async fn insert_task_run(
    pool: &SqlitePool,
    task_id: &str,
    status: &str,
    file_size_bytes: Option<i64>,
    file_path: Option<&str>,
    error: Option<&str>,
    anomaly_flags: Option<&str>,
    server_run_id: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO task_runs (task_id, run_at, status, file_size_bytes, file_path, error, anomaly_flags, server_run_id)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(task_id)
    .bind(Utc::now().to_rfc3339())
    .bind(status)
    .bind(file_size_bytes)
    .bind(file_path)
    .bind(error)
    .bind(anomaly_flags)
    .bind(server_run_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_latest_run_for_task(
    pool: &SqlitePool,
    task_id: &str,
) -> anyhow::Result<Option<crate::models::task_run::TaskRun>> {
    sqlx::query_as::<_, crate::models::task_run::TaskRun>(
        r#"
        SELECT id, task_id, run_at, status, file_size_bytes, file_path, error, anomaly_flags, server_run_id
        FROM task_runs WHERE task_id = ? ORDER BY run_at DESC LIMIT 1
        "#,
    )
    .bind(task_id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

/// Last run that downloaded a file (dir_sync pass / no_changes runs have size 0).
pub async fn get_latest_download_bytes_for_task(
    pool: &SqlitePool,
    task_id: &str,
) -> anyhow::Result<Option<i64>> {
    let row: Option<(Option<i64>,)> = sqlx::query_as(
        r#"
        SELECT file_size_bytes FROM task_runs
        WHERE task_id = ? AND file_size_bytes IS NOT NULL AND file_size_bytes > 0
        ORDER BY run_at DESC LIMIT 1
        "#,
    )
    .bind(task_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|(b,)| b))
}

#[derive(Debug, Clone, Default)]
pub struct TaskRunStats {
    pub fail_count: i64,
    pub warn_count: i64,
}

pub async fn get_task_run_stats(
    pool: &SqlitePool,
    task_id: &str,
) -> anyhow::Result<TaskRunStats> {
    let row: (Option<i64>, Option<i64>) = sqlx::query_as(
        r#"
        SELECT
            SUM(CASE WHEN status = 'fail' THEN 1 ELSE 0 END),
            SUM(CASE
                WHEN status = 'warn'
                  OR (anomaly_flags IS NOT NULL AND TRIM(anomaly_flags) != '')
                THEN 1 ELSE 0
            END)
        FROM task_runs
        WHERE task_id = ?
        "#,
    )
    .bind(task_id)
    .fetch_one(pool)
    .await?;

    Ok(TaskRunStats {
        fail_count: row.0.unwrap_or(0),
        warn_count: row.1.unwrap_or(0),
    })
}
