//! Task type normalization and filename labels.

pub fn normalize(task_type: &str) -> &str {
    match task_type {
        "file_archive" => "files_archive",
        other => other,
    }
}

pub fn file_label(task_type: &str) -> &str {
    match normalize(task_type) {
        "mysql_dump" | "mariadb_dump" => "mysql",
        "postgresql_dump" => "postgresql",
        "sqlite_dump" => "sqlite",
        "files_archive" => "files",
        "dir_sync" => "sync",
        "shell" => "shell",
        other => other,
    }
}
