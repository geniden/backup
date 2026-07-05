//! Standard backup filename pattern.

use chrono::Local;

use crate::task_types;

pub fn backup_filename(task_name: &str, task_type: &str, extension: &str) -> String {
    let label = task_types::file_label(task_type);
    let now = Local::now();
    format!(
        "backup_{}_{}_{}_{}.{}",
        task_name,
        label,
        now.format("%Y-%m-%d"),
        now.format("%H%M%S"),
        extension
    )
}

/// `backup_*_mysql_2026-06-28_120000.sql` → `backup_*_mysql_2026-06-28_120000.zip`
pub fn archive_zip_name(source_filename: &str) -> String {
    let stem = source_filename
        .rsplit_once('.')
        .map(|(name, _)| name)
        .unwrap_or(source_filename);
    format!("{stem}.zip")
}

pub fn dir_sync_zip_name(task_name: &str) -> String {
    backup_filename(task_name, "dir_sync", "zip")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_sync_uses_unified_backup_prefix() {
        let name = dir_sync_zip_name("bogorodsk");
        assert!(name.starts_with("backup_bogorodsk_sync_"));
        assert!(name.ends_with(".zip"));
    }
}
