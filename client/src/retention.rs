//! Local backup file retention per connection (cooldown, keep-latest).

use std::collections::HashMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use tracing::{debug, info};

use crate::db;
use crate::i18n;
use crate::models::connection::Connection;

const COOLDOWN_SECS: i64 = 24 * 60 * 60;

/// Menu options: 0 = never delete.
pub const RETENTION_OPTIONS: &[i32] = &[0, 7, 14, 30, 60];

pub fn retention_label(days: i32) -> String {
    match days {
        0 => i18n::t("retention.never"),
        7 => i18n::t("retention.days_7"),
        14 => i18n::t("retention.days_14"),
        30 => i18n::t("retention.days_30"),
        60 => i18n::t("retention.days_60"),
        _ => i18n::t("retention.custom"),
    }
}

/// After a successful backup download: enforce retention at most once per 24h per connection.
pub async fn maybe_run_after_success(
    pool: &SqlitePool,
    connection_id: &str,
    slug: &str,
) -> anyhow::Result<()> {
    let Some(conn) = db::get_connection(pool, connection_id).await? else {
        return Ok(());
    };

    if !conn.enabled {
        debug!("[{slug}] Retention: skipped (connection disabled)");
        return Ok(());
    }
    if conn.retention_days <= 0 {
        debug!("[{slug}] Retention: skipped (policy 0 = never)");
        return Ok(());
    }
    if !cooldown_elapsed(conn.retention_last_run.as_deref()) {
        debug!("[{slug}] Retention: skipped (cooldown)");
        return Ok(());
    }

    let stats = enforce(pool, &conn).await?;
    db::touch_retention_last_run(pool, connection_id).await?;

    if stats.removed > 0 {
        info!(
            "[{slug}] Retention: finished — removed {} file(s), freed {}",
            stats.removed,
            crate::format::format_bytes(stats.bytes_freed)
        );
    } else {
        debug!("[{slug}] Retention: nothing to remove");
    }

    Ok(())
}

struct RemovalStats {
    removed: usize,
    bytes_freed: u64,
}

async fn enforce(pool: &SqlitePool, conn: &Connection) -> anyhow::Result<RemovalStats> {
    let rows = db::list_retention_runs(pool, &conn.id).await?;
    let policy = i64::from(conn.retention_days);
    let mut per_task_count: HashMap<String, usize> = HashMap::new();
    for row in &rows {
        *per_task_count.entry(row.task_id.clone()).or_insert(0) += 1;
    }

    let mut stats = RemovalStats {
        removed: 0,
        bytes_freed: 0,
    };
    let mut seen_latest: HashMap<String, bool> = HashMap::new();

    for row in rows {
        if crate::validation::normalize_task_type(&row.task_type) == "dir_sync" {
            continue;
        }

        let is_first_for_task = !seen_latest.contains_key(&row.task_id);
        if is_first_for_task {
            seen_latest.insert(row.task_id.clone(), true);
            let age = run_age_days(&row.run_at).unwrap_or(0);
            if per_task_count.get(&row.task_id).copied().unwrap_or(0) == 1 && age >= policy {
                info!(
                    "[{}] Retention: kept {} (latest/only backup for task {})",
                    conn.slug, row.file_path, row.task_name
                );
            }
            continue;
        }

        let Some(age) = run_age_days(&row.run_at) else {
            continue;
        };
        if age < policy {
            continue;
        }

        let path = Path::new(&row.file_path);
        if path.exists() {
            if let Ok(meta) = std::fs::metadata(path) {
                stats.bytes_freed += meta.len();
            }
            if let Err(e) = std::fs::remove_file(path) {
                tracing::warn!(
                    "[{}] Retention: could not delete {}: {}",
                    conn.slug,
                    row.file_path,
                    e
                );
                continue;
            }
        }

        db::clear_task_run_file_path(pool, row.id).await?;
        stats.removed += 1;
        info!(
            "[{}] Retention: deleted {} (task {}, age {}d, policy {}d)",
            conn.slug, row.file_path, row.task_name, age, policy
        );
    }

    Ok(stats)
}

fn cooldown_elapsed(last_run: Option<&str>) -> bool {
    let Some(raw) = last_run else {
        return true;
    };
    let Ok(parsed) = DateTime::parse_from_rfc3339(raw) else {
        return true;
    };
    let elapsed = Utc::now().signed_duration_since(parsed.with_timezone(&Utc));
    elapsed.num_seconds() >= COOLDOWN_SECS
}

fn run_age_days(run_at: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(run_at)
        .ok()
        .map(|dt| {
            Utc::now()
                .signed_duration_since(dt.with_timezone(&Utc))
                .num_days()
        })
}
