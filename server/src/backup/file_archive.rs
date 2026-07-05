//! Zip archive of configured files/directories.

use anyhow::Result;
use std::path::Path;
use tracing::{debug, warn};
use walkdir::WalkDir;
use zip::write::FileOptions;
use zip::ZipWriter;
use serde::Deserialize;
use crate::paths;
use crate::utils::{self, hash_file_sha256};

use super::naming;
use super::zip_entries::{self, ZipBuildStats};
use super::TaskResult;

#[derive(Deserialize)]
pub struct FileArchiveTask {
    pub source_path: String,
    #[serde(rename = "ignore")]
    pub ignore_patterns: Vec<String>,
    pub files_dir: String,
}

impl FileArchiveTask {
    pub async fn execute(&self, task_name: &str, task_type: &str) -> Result<TaskResult> {
        if !Path::new(&self.source_path).exists() {
            return Err(anyhow::anyhow!(
                "Source path does not exist: {}",
                self.source_path
            ));
        }

        let filename = naming::backup_filename(task_name, task_type, "zip");
        let filepath = paths::join(&self.files_dir, &filename);

        let source_path = self.source_path.clone();
        let filepath_clone = filepath.to_string_lossy().into_owned();
        let ignore_patterns = self.ignore_patterns.clone();
        let filepath_for_hash = filepath_clone.clone();

        let (zip_size, zip_stats) = tokio::task::spawn_blocking(move || {
            Self::create_archive(&source_path, &filepath_clone, &ignore_patterns)
        })
        .await??;

        if zip_stats.files_added == 0 {
            let _ = std::fs::remove_file(&filepath);
            anyhow::bail!("files_archive: no files archived (empty tree or all files skipped)");
        }

        if zip_stats.files_skipped > 0 {
            warn!(
                "files_archive: skipped {} file(s), archived {}",
                zip_stats.files_skipped,
                zip_stats.files_added
            );
        }

        let file_hash = hash_file_sha256(&filepath_for_hash).unwrap_or_else(|e| {
            warn!("Could not hash '{}': {}", filepath_for_hash, e);
            "unknown".to_string()
        });

        debug!(
            "Archive {} ({}, {} files)",
            filename,
            utils::format_bytes(zip_size),
            zip_stats.files_added
        );

        Ok(TaskResult {
            filename,
            size_bytes: zip_size as i64,
            files_count: zip_stats.files_added,
            file_hash,
        })
    }

    fn is_ignored(relative: &Path, patterns: &[String]) -> bool {
        let path_str = relative.to_string_lossy().replace('\\', "/");

        for pattern in patterns {
            if pattern.contains('/') {
                let pattern_clean = pattern.trim_end_matches('/');
                if path_str.starts_with(pattern_clean)
                    || path_str.contains(&format!("/{}/", pattern_clean))
                    || path_str == pattern_clean
                {
                    return true;
                }
            } else if pattern.starts_with("*.") {
                let ext = &pattern[2..];
                if relative
                    .extension()
                    .map(|e| e.to_string_lossy() == ext)
                    .unwrap_or(false)
                {
                    return true;
                }
            } else if path_str.split('/').any(|component| component == pattern) {
                return true;
            }
        }

        false
    }

    fn create_archive(
        source_path: &str,
        filepath: &str,
        ignore_patterns: &[String],
    ) -> Result<(u64, ZipBuildStats)> {
        let file = std::fs::File::create(filepath)?;
        let mut zip = ZipWriter::new(file);
        let options = FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        let mut stats = ZipBuildStats::default();
        let source = Path::new(source_path);

        for entry in WalkDir::new(source).follow_links(true).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();

            if path.is_dir() {
                continue;
            }

            let relative = match path.strip_prefix(source) {
                Ok(r) => r,
                Err(e) => {
                    warn!("files_archive skip: {} — {e}", path.display());
                    continue;
                }
            };

            if Self::is_ignored(relative, ignore_patterns) {
                debug!("Ignored: {:?}", relative);
                continue;
            }

            let one = zip_entries::try_append_file(&mut zip, path, relative, options);
            stats.merge(one);
        }

        zip.finish()?;
        let zip_size = std::fs::metadata(filepath)?.len();
        Ok((zip_size, stats))
    }
}
