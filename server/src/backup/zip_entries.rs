//! Per-file ZIP append: skip unreadable paths, bad names, or huge files (RAM).

use std::fs::File;
use std::io::Write;
use std::path::Path;
use tracing::warn;
use zip::write::FileOptions;
use zip::ZipWriter;

use crate::utils;

/// Each file is read fully into RAM before writing to the ZIP.
pub const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Debug, Default, Clone, Copy)]
pub struct ZipBuildStats {
    pub files_added: u64,
    pub uncompressed_bytes: u64,
    pub files_skipped: u64,
}

impl ZipBuildStats {
    pub fn merge(&mut self, other: ZipBuildStats) {
        self.files_added += other.files_added;
        self.uncompressed_bytes += other.uncompressed_bytes;
        self.files_skipped += other.files_skipped;
    }
}

/// ZIP entry path: UTF-8 required (Cyrillic is OK). Rejects `..`, NUL, non-UTF-8.
pub fn relative_zip_name(relative: &Path) -> Option<String> {
    let name = relative.to_str()?;
    if name.is_empty() || name.contains('\0') {
        return None;
    }
    let normalized = name.replace('\\', "/");
    if normalized.ends_with('/') {
        return None;
    }
    if normalized
        .split('/')
        .any(|part| part == ".." || part.contains('\0'))
    {
        return None;
    }
    Some(normalized)
}

/// Append one file; on failure log and count as skipped (archive continues).
pub fn try_append_file(
    zip: &mut ZipWriter<File>,
    absolute: &Path,
    relative: &Path,
    options: FileOptions,
) -> ZipBuildStats {
    let mut stats = ZipBuildStats::default();

    let Some(entry_name) = relative_zip_name(relative) else {
        warn!("ZIP skip (unsupported entry name): {}", absolute.display());
        stats.files_skipped = 1;
        return stats;
    };

    let meta = match std::fs::metadata(absolute) {
        Ok(m) => m,
        Err(e) => {
            warn!("ZIP skip (metadata): {} — {e}", absolute.display());
            stats.files_skipped = 1;
            return stats;
        }
    };

    if !meta.is_file() {
        stats.files_skipped = 1;
        return stats;
    }

    let len = meta.len();
    if len > MAX_FILE_BYTES {
        warn!(
            "ZIP skip (>{} RAM limit): {} ({})",
            utils::format_bytes(MAX_FILE_BYTES),
            absolute.display(),
            utils::format_bytes(len)
        );
        stats.files_skipped = 1;
        return stats;
    }

    let contents = match std::fs::read(absolute) {
        Ok(c) => c,
        Err(e) => {
            warn!("ZIP skip (read): {} — {e}", absolute.display());
            stats.files_skipped = 1;
            return stats;
        }
    };

    if let Err(e) = zip.start_file(&entry_name, options) {
        warn!("ZIP skip (entry): {entry_name} — {e}");
        stats.files_skipped = 1;
        return stats;
    }

    if let Err(e) = zip.write_all(&contents) {
        warn!("ZIP skip (zip write): {entry_name} — {e}");
        stats.files_skipped = 1;
        return stats;
    }

    stats.files_added = 1;
    stats.uncompressed_bytes = contents.len() as u64;
    stats
}
