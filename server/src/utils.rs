//! SHA256 hash helper, byte sizes, and cross-platform PATH lookup.

use anyhow::Result;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;
use std::process::Command;

pub fn format_bytes(bytes: u64) -> String {
    let b = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", b / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", b / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", b / (1024.0 * 1024.0 * 1024.0))
    }
}

pub fn hash_file_sha256(path: &str) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];

    while let Ok(bytes_read) = file.read(&mut buffer) {
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

/// True if the value looks like a filesystem path (not a bare command name).
pub fn is_explicit_tool_path(path_or_name: &str) -> bool {
    let path_or_name = path_or_name.trim();
    path_or_name.contains(std::path::MAIN_SEPARATOR)
        || path_or_name.starts_with("./")
        || path_or_name.starts_with("../")
        || path_or_name.starts_with('~')
        || (cfg!(windows)
            && path_or_name.len() >= 2
            && path_or_name.as_bytes()[1] == b':')
}

/// True if `path_or_name` exists on disk or resolves on the system PATH.
pub fn command_available(path_or_name: &str) -> bool {
    let path_or_name = path_or_name.trim();
    if path_or_name.is_empty() {
        return false;
    }
    if is_explicit_tool_path(path_or_name) {
        return explicit_path_exists(Path::new(path_or_name));
    }
    command_in_path(path_or_name)
}

#[cfg(windows)]
fn explicit_path_exists(p: &Path) -> bool {
    p.exists() || try_windows_exe_suffix(p)
}

#[cfg(not(windows))]
fn explicit_path_exists(p: &Path) -> bool {
    p.exists()
}

#[cfg(windows)]
fn try_windows_exe_suffix(path: &Path) -> bool {
    if path.extension().is_some() {
        return false;
    }
    path.with_extension("exe").exists()
}

fn command_in_path(name: &str) -> bool {
    #[cfg(windows)]
    {
        Command::new("where")
            .arg(name)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        Command::new("which")
            .arg(name)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}