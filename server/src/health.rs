//! Startup checks for mysqldump, pg_dump, disk space.

use std::path::Path;

use tracing::warn;

use crate::config::Config;
use crate::utils;

pub fn run_health_checks(config: &Config) {
    if let Ok(dir) = config.files_dir_abs() {
        ensure_writable(&dir);
    }

    // Archives use the Rust `zip` crate — no external `zip` binary required.

    if cfg!(windows) {
        check_tool(
            &config.mysqldump_path,
            "Set mysqldump_path in config.toml to the full path of mysqldump.exe, or add mysqldump to PATH",
        );
        check_tool(
            &config.pg_dump_path,
            "Install PostgreSQL client tools, set pg_dump_path in config.toml, or add pg_dump to PATH",
        );
    } else {
        check_tool(
            &config.mysqldump_path,
            "Install: sudo apt install mysql-client (or set mysqldump_path in config.toml)",
        );
        check_tool(
            &config.pg_dump_path,
            "Install: sudo apt install postgresql-client (or set pg_dump_path in config.toml)",
        );
    }
}

fn ensure_writable(path: &Path) {
    let test = path.join(".writable_test");
    match std::fs::write(&test, b"x") {
        Ok(_) => {
            let _ = std::fs::remove_file(test);
        }
        Err(e) => warn!("Directory not writable ({}): {}", crate::paths::display_path(path), e),
    }
}

fn check_tool(path: &str, hint: &str) {
    if utils::command_available(path) {
        return;
    }
    let tool_label = if utils::is_explicit_tool_path(path) {
        crate::paths::display_path(Path::new(path))
    } else {
        path.to_string()
    };
    warn!("Missing tool '{}'. {}", tool_label, hint);
}
