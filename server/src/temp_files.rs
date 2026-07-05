//! Safe temp file paths and cleanup after download.

use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use tracing::{debug, warn};

pub fn is_safe_filename(filename: &str) -> bool {
    if filename.is_empty() {
        return false;
    }

    let dangerous = ["..", "\\", "//", "\\\\", "%2e%2e", "%2f", "%5c"];
    let lower = filename.to_lowercase();
    for pattern in dangerous {
        if lower.contains(pattern) {
            return false;
        }
    }

    for component in Path::new(filename).components() {
        if !matches!(component, Component::Normal(_)) {
            return false;
        }
    }

    true
}

pub fn resolve_temp_file(files_dir: &str, filename: &str) -> Result<PathBuf> {
    if !is_safe_filename(filename) {
        anyhow::bail!("Invalid filename");
    }

    let files_dir_canonical = std::fs::canonicalize(files_dir)
        .with_context(|| format!("Cannot canonicalize files_dir: {files_dir}"))?;
    let filepath = PathBuf::from(files_dir).join(filename);
    let filepath_canonical = std::fs::canonicalize(&filepath)
        .with_context(|| format!("File not found: {filename}"))?;

    if !filepath_canonical.starts_with(&files_dir_canonical) {
        anyhow::bail!("Access denied");
    }

    Ok(filepath_canonical)
}

pub async fn delete_temp_file(files_dir: &str, filename: &str) -> Result<()> {
    let path = resolve_temp_file(files_dir, filename)?;
    let metadata = tokio::fs::metadata(&path).await?;
    if !metadata.is_file() {
        anyhow::bail!("Not a file: {filename}");
    }
    tokio::fs::remove_file(&path).await?;
    debug!("Deleted temp file: {}", filename);
    Ok(())
}

pub async fn delete_temp_file_best_effort(files_dir: &str, filename: &str) {
    if let Err(e) = delete_temp_file(files_dir, filename).await {
        warn!("Could not delete temp file {}: {}", filename, e);
    }
}
