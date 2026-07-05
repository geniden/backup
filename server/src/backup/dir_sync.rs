//! Incremental directory sync: zip files after cursor `(sync_last, sync_last_path)` in batches.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use sqlx::SqlitePool;
use tracing::{debug, info, warn};
use walkdir::WalkDir;
use zip::write::FileOptions;
use zip::ZipWriter;

use crate::db::{self, DirSyncCursor};
use crate::paths;
use crate::utils::{self, hash_file_sha256};

use super::naming;
use super::zip_entries::{self, ZipBuildStats};
use super::{TaskOutcome, TaskResult};

pub const DEFAULT_MAX_BATCH_MB: u64 = 200;
pub const MAX_BATCH_MB_HARD_CAP: u64 = 500;

pub async fn execute(
    def_id: &str,
    task_name: &str,
    data: &serde_json::Value,
    db_pool: &SqlitePool,
) -> Result<TaskOutcome> {
    let source_path = data
        .get("source_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing source_path"))?;

    if !Path::new(source_path).exists() {
        anyhow::bail!("Source path does not exist: {source_path}");
    }

    let files_dir = data
        .get("files_dir")
        .and_then(|v| v.as_str())
        .unwrap_or("./data/temp");

    let first_sync_days = data
        .get("first_sync_days")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as i64;

    let max_batch_mb = parse_max_batch_mb(data);
    let max_batch_bytes = max_batch_mb * 1024 * 1024;

    let cursor = db::get_dir_sync_cursor(db_pool, def_id).await?;
    log_missing_anchor(source_path, &cursor);

    let min_mtime = min_mtime_for_scan(&cursor, first_sync_days);

    let source_owned = source_path.to_string();
    let cursor_for_scan = cursor.clone();
    let scan = tokio::task::spawn_blocking(move || {
        scan_batch_files(&source_owned, &cursor_for_scan, min_mtime, max_batch_bytes)
    })
    .await
    .context("dir_sync scan join")??;

    if scan.files.is_empty() {
        let detail = if source_has_any_file(source_path) {
            "up_to_date"
        } else {
            "empty"
        };
        info!(
            "{task_name} | dir_sync | no files after cursor ({}, '{}') [{detail}]",
            cursor.sync_last, cursor.sync_last_path
        );
        return Ok(TaskOutcome::NoChanges {
            detail: Some(detail),
        });
    }

    let filename = naming::dir_sync_zip_name(task_name);
    let filepath = paths::join(files_dir, &filename);
    let filepath = filepath.to_string_lossy().into_owned();
    let filepath_for_hash = filepath.clone();
    let files = scan.files;
    let new_cursor = scan.new_cursor;
    let candidate_count = files.len();

    let filepath_for_zip = filepath.clone();
    let (zip_size, zip_stats) = tokio::task::spawn_blocking(move || {
        create_incremental_zip(&filepath_for_zip, &files)
    })
    .await
    .context("dir_sync zip join")??;

    if zip_stats.files_added == 0 {
        let _ = std::fs::remove_file(&filepath);
        anyhow::bail!(
            "dir_sync: all {candidate_count} file(s) skipped (unreadable names, I/O errors, or >{} each)",
            utils::format_bytes(zip_entries::MAX_FILE_BYTES)
        );
    }

    if zip_stats.files_skipped > 0 {
        warn!(
            "{task_name} | dir_sync | skipped {} file(s), archived {}",
            zip_stats.files_skipped,
            zip_stats.files_added
        );
    }

    let file_hash = hash_file_sha256(&filepath_for_hash).unwrap_or_else(|e| {
        warn!("Could not hash '{}': {}", filepath_for_hash, e);
        "unknown".to_string()
    });

    db::insert_dir_sync_pending(
        db_pool,
        &filename,
        def_id,
        new_cursor.sync_last,
        &new_cursor.sync_last_path,
    )
    .await?;

    debug!(
        "dir_sync {filename}: {} file(s), {}, cursor ({}, '{}')",
        zip_stats.files_added,
        utils::format_bytes(zip_size),
        new_cursor.sync_last,
        new_cursor.sync_last_path
    );

    Ok(TaskOutcome::File(TaskResult {
        filename,
        size_bytes: zip_size as i64,
        files_count: zip_stats.files_added,
        file_hash,
    }))
}

struct ScannedFile {
    relative: PathBuf,
    relative_key: String,
    absolute: PathBuf,
    mtime: i64,
    size: u64,
}

struct ScanResult {
    files: Vec<ScannedFile>,
    new_cursor: DirSyncCursor,
}

fn parse_max_batch_mb(data: &serde_json::Value) -> u64 {
    let mb = data
        .get("max_batch_mb")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_MAX_BATCH_MB);
    mb.clamp(1, MAX_BATCH_MB_HARD_CAP)
}

fn min_mtime_for_scan(cursor: &DirSyncCursor, first_sync_days: i64) -> i64 {
    if cursor.sync_last > 0 || !cursor.sync_last_path.is_empty() {
        return 0;
    }
    if first_sync_days <= 0 {
        return 0;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    now - first_sync_days * 86_400
}

fn log_missing_anchor(source_path: &str, cursor: &DirSyncCursor) {
    if cursor.sync_last_path.is_empty() {
        return;
    }
    let anchor = Path::new(source_path).join(&cursor.sync_last_path);
    if !anchor.exists() {
        info!(
            "dir_sync: cursor anchor file missing ({}), continuing from cursor without it",
            cursor.sync_last_path
        );
    }
}

fn normalize_rel_path(relative: &Path) -> String {
    relative.to_string_lossy().replace('\\', "/")
}

fn source_has_any_file(source_path: &str) -> bool {
    WalkDir::new(source_path)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .any(|e| e.path().is_file())
}

/// Lexicographic order: files strictly after `(sync_last, sync_last_path)`.
pub fn is_after_cursor(mtime: i64, path_key: &str, cursor: &DirSyncCursor) -> bool {
    if mtime > cursor.sync_last {
        return true;
    }
    if mtime < cursor.sync_last {
        return false;
    }
    path_key > cursor.sync_last_path.as_str()
}

fn scan_batch_files(
    source_path: &str,
    cursor: &DirSyncCursor,
    min_mtime: i64,
    max_batch_bytes: u64,
) -> Result<ScanResult> {
    let source = Path::new(source_path);
    let mut candidates = Vec::new();

    for entry in WalkDir::new(source).follow_links(false).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            continue;
        }

        let relative = match path.strip_prefix(source) {
            Ok(r) => r.to_path_buf(),
            Err(e) => {
                warn!("dir_sync scan skip: {} — {e}", path.display());
                continue;
            }
        };

        let relative_key = normalize_rel_path(&relative);

        let mtime = match file_mtime_secs(path) {
            Ok(t) => t,
            Err(e) => {
                warn!("dir_sync scan skip: {} — {e}", path.display());
                continue;
            }
        };

        if min_mtime > 0 && mtime < min_mtime {
            continue;
        }

        if !is_after_cursor(mtime, &relative_key, cursor) {
            continue;
        }

        let size = match std::fs::metadata(path) {
            Ok(m) => m.len(),
            Err(e) => {
                warn!("dir_sync scan skip: {} — {e}", path.display());
                continue;
            }
        };

        candidates.push(ScannedFile {
            relative,
            relative_key,
            absolute: path.to_path_buf(),
            mtime,
            size,
        });
    }

    candidates.sort_by(|a, b| {
        a.mtime
            .cmp(&b.mtime)
            .then_with(|| a.relative_key.cmp(&b.relative_key))
    });

    let mut files = Vec::new();
    let mut batch_bytes: u64 = 0;

    for item in candidates {
        if !files.is_empty() && batch_bytes.saturating_add(item.size) > max_batch_bytes {
            break;
        }
        batch_bytes = batch_bytes.saturating_add(item.size);
        files.push(item);
    }

    let new_cursor = files
        .last()
        .map(|last| DirSyncCursor {
            sync_last: last.mtime,
            sync_last_path: last.relative_key.clone(),
        })
        .unwrap_or_else(|| cursor.clone());

    Ok(ScanResult { files, new_cursor })
}

fn file_mtime_secs(path: &Path) -> Result<i64> {
    let mtime = std::fs::metadata(path)
        .with_context(|| format!("metadata {}", path.display()))?
        .modified()
        .with_context(|| format!("mtime {}", path.display()))?;
    Ok(mtime
        .duration_since(UNIX_EPOCH)
        .with_context(|| format!("mtime epoch {}", path.display()))?
        .as_secs() as i64)
}

fn create_incremental_zip(filepath: &str, files: &[ScannedFile]) -> Result<(u64, ZipBuildStats)> {
    let file = std::fs::File::create(filepath)?;
    let mut zip = ZipWriter::new(file);
    let options = FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let mut stats = ZipBuildStats::default();
    for item in files {
        let one = zip_entries::try_append_file(
            &mut zip,
            &item.absolute,
            &item.relative,
            options,
        );
        stats.merge(one);
    }

    zip.finish()?;
    let zip_size = std::fs::metadata(filepath)?.len();
    Ok((zip_size, stats))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_orders_by_mtime_then_path() {
        let cursor = DirSyncCursor {
            sync_last: 100,
            sync_last_path: "b.txt".into(),
        };
        assert!(!is_after_cursor(99, "z.txt", &cursor));
        assert!(!is_after_cursor(100, "a.txt", &cursor));
        assert!(!is_after_cursor(100, "b.txt", &cursor));
        assert!(is_after_cursor(100, "c.txt", &cursor));
        assert!(is_after_cursor(101, "a.txt", &cursor));
    }

    #[test]
    fn max_batch_mb_clamped() {
        let data = serde_json::json!({ "max_batch_mb": 999 });
        assert_eq!(parse_max_batch_mb(&data), MAX_BATCH_MB_HARD_CAP);
        let data = serde_json::json!({});
        assert_eq!(parse_max_batch_mb(&data), DEFAULT_MAX_BATCH_MB);
    }
}
