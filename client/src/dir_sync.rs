//! dir_sync helpers: last received archive name, local reset.

use std::path::{Path, PathBuf};

use sqlx::SqlitePool;

use crate::db;
use crate::models::connection::Connection;
use crate::models::task::Task;
use crate::paths;
use crate::validation;

/// Basename of the latest dir_sync archive on disk (`backup_{task}_sync_*` or legacy `sync_{task}_*`).
pub fn latest_sync_archive_on_disk(backups_root: &Path, slug: &str, task_name: &str) -> Option<String> {
    let dir = paths::connection_backups_dir_with(backups_root, slug);
    let prefixes = [
        format!("backup_{task_name}_sync_"),
        format!("sync_{task_name}_"),
    ];
    let mut best: Option<String> = None;

    let entries = std::fs::read_dir(&dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".zip") {
            continue;
        }
        if prefixes.iter().any(|p| name.starts_with(p)) {
            if best.as_ref().is_none_or(|b| name > *b) {
                best = Some(name);
            }
        }
    }
    best
}

fn filename_from_path(path: &str) -> Option<String> {
    PathBuf::from(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
}

/// Latest received dir_sync archive: DB run history first, then on-disk scan.
pub async fn last_received_filename(
    pool: &SqlitePool,
    conn: &Connection,
    task: &Task,
) -> anyhow::Result<Option<String>> {
    if validation::normalize_task_type(&task.task_type) != "dir_sync" {
        return Ok(None);
    }

    if let Some(path) = db::get_latest_sync_file_path(pool, &task.id).await? {
        if let Some(name) = filename_from_path(&path) {
            return Ok(Some(name));
        }
    }

    let custom = db::backups_root_custom(pool).await?;
    let root = paths::resolve_backups_root(custom.as_deref());
    Ok(latest_sync_archive_on_disk(&root, &conn.slug, &task.task_name))
}

/// Clear local run history for a dir_sync task (does not delete ZIP files on disk).
pub async fn reset_local_state(pool: &SqlitePool, task_id: &str) -> anyhow::Result<()> {
    db::clear_task_run_history(pool, task_id).await
}
