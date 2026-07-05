//! Server paths: temp, scripts, data from config.

use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};

const CONFIG_FILE: &str = "config.toml";
const MAX_CONFIG_SEARCH_DEPTH: usize = 6;

pub fn config_path() -> Result<PathBuf> {
    Ok(find_config_file().unwrap_or_else(|| {
        default_config_path().unwrap_or_else(|_| PathBuf::from(CONFIG_FILE))
    }))
}

/// Directory that contains `config.toml` (and `tls/`, `data/`).
pub fn base_dir() -> Result<PathBuf> {
    Ok(config_path()?
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".")))
}

pub fn db_path() -> Result<PathBuf> {
    Ok(base_dir()?.join("backup.db"))
}

/// Search `config.toml`: current dir → parents → exe dir → parents (for `cargo run` / `target/debug` layout).
fn find_config_file() -> Option<PathBuf> {
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(p) = search_config_in_parents(&cwd) {
            return Some(p);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            if let Some(p) = search_config_in_parents(parent) {
                return Some(p);
            }
        }
    }
    None
}

fn search_config_in_parents(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    for _ in 0..MAX_CONFIG_SEARCH_DEPTH {
        let candidate = dir.join(CONFIG_FILE);
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

fn default_config_path() -> Result<PathBuf> {
    let exe_dir = std::env::current_exe()
        .context("Failed to get executable path")?
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow::anyhow!("Cannot determine executable directory"))?;
    Ok(exe_dir.join(CONFIG_FILE))
}

/// Resolve a config-relative or absolute path (native separators, no mixed slashes).
pub fn resolve(relative: &str) -> Result<PathBuf> {
    let rel = Path::new(relative);
    if rel.is_absolute() {
        return Ok(rel.to_path_buf());
    }
    let mut base = base_dir()?;
    for component in rel.components() {
        match component {
            Component::Normal(c) => base.push(c),
            Component::CurDir => {}
            Component::ParentDir => {
                base.pop();
            }
            Component::RootDir | Component::Prefix(_) => return Ok(rel.to_path_buf()),
        }
    }
    Ok(base)
}

/// Join `base` and `name` using the platform path API (safe on Windows).
pub fn join(base: impl AsRef<Path>, name: &str) -> PathBuf {
    base.as_ref().join(name)
}

/// User-facing path string with native separators (no `C:\foo\bar/baz` mix).
pub fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('/', std::path::MAIN_SEPARATOR_STR)
}

/// SQLite connection URL (forward slashes — required on Windows).
pub fn sqlite_url(db_path: &Path) -> String {
    format!("sqlite:{}", db_path.to_string_lossy().replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_finds_config_in_parent() {
        let tmp = std::env::temp_dir().join(format!("backup-server-paths-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("target").join("debug")).unwrap();
        std::fs::write(tmp.join("config.toml"), "device_id = \"\"\n").unwrap();
        let found = search_config_in_parents(&tmp.join("target").join("debug"));
        assert_eq!(found, Some(tmp.join("config.toml")));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
