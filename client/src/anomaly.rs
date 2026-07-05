//! Flags suspicious backup sizes (tiny archive, large drop vs last OK run).

/// Empty or failed archives are usually well under 1 KB; compressed DB dumps can be tens of KB.
const TINY_ARCHIVE_BYTES: u64 = 1024;
const DROP_RATIO: f64 = 0.5;

pub fn detect(task_type: &str, file_size: u64, prev_ok_size: Option<u64>) -> Vec<String> {
    let normalized = crate::validation::normalize_task_type(task_type);
    if normalized == "shell" {
        return Vec::new();
    }

    let mut flags = Vec::new();

    if is_archive_type(normalized) && file_size < TINY_ARCHIVE_BYTES {
        flags.push("tiny_file".into());
    }

    if let Some(prev) = prev_ok_size {
        if prev > 0 {
            let ratio = file_size as f64 / prev as f64;
            if ratio < DROP_RATIO {
                flags.push("size_drop".into());
            }
        }
    }

    flags
}

pub fn flags_to_string(flags: &[String]) -> Option<String> {
    if flags.is_empty() {
        None
    } else {
        Some(flags.join(","))
    }
}

fn is_archive_type(task_type: &str) -> bool {
    matches!(
        task_type,
        "mysql_dump" | "postgresql_dump" | "db_dump" | "files_archive" | "file_archive"
            | "dir_sync"
    )
}
