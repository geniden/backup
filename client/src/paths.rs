//! Paths relative to executable (db, data, backups).

use std::path::{Path, PathBuf};

/// Default backups root relative to the client install folder.
pub const DEFAULT_BACKUPS_DIR: &str = "data/backups";

pub fn app_root() -> PathBuf {
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        return PathBuf::from(manifest);
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn db_path() -> PathBuf {
    app_root().join("backup.db")
}

pub fn data_dir() -> PathBuf {
    app_root().join("data")
}

pub fn log_path() -> PathBuf {
    data_dir().join("backup.log")
}

pub fn backups_root() -> PathBuf {
    resolve_backups_root(None)
}

/// `custom` empty/None → `{app_root}/data/backups`; otherwise the configured absolute path.
pub fn resolve_backups_root(custom: Option<&str>) -> PathBuf {
    match custom.map(str::trim).filter(|s| !s.is_empty()) {
        None => app_root().join(DEFAULT_BACKUPS_DIR),
        Some(path) => PathBuf::from(path),
    }
}

pub fn connection_backups_dir_with(backups_root: &Path, slug: &str) -> PathBuf {
    backups_root.join(slug)
}

pub fn ensure_layout() -> anyhow::Result<()> {
    std::fs::create_dir_all(data_dir())?;
    std::fs::create_dir_all(backups_root())?;
    std::fs::create_dir_all(data_dir().join("ca"))?;
    Ok(())
}

pub fn ensure_backups_root_exists(root: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(root)?;
    Ok(())
}

pub fn validate_backups_root_writable(root: &Path) -> anyhow::Result<()> {
    ensure_backups_root_exists(root)?;
    let test = root.join(".writable_test");
    std::fs::write(&test, b"x")?;
    std::fs::remove_file(test)?;
    Ok(())
}

/// True if the path contains characters outside ASCII and Cyrillic (e.g. CJK).
pub fn path_has_extended_unicode(path: &str) -> bool {
    path.chars().any(|c| {
        !c.is_ascii() && !(('\u{0400}'..='\u{04FF}').contains(&c) || c == 'ё' || c == 'Ё')
    })
}

/// User-facing note when a custom backups root uses non-ASCII characters.
pub fn backups_path_unicode_warning(path: &str) -> Option<String> {
    if path_has_extended_unicode(path) {
        Some(crate::i18n::t("settings.backups_unicode_warn"))
    } else {
        None
    }
}

/// User-facing label for Settings (relative default for portability).
pub fn backups_root_label(custom: Option<&str>) -> String {
    match custom.map(str::trim).filter(|s| !s.is_empty()) {
        None => format!(
            "{} {}",
            DEFAULT_BACKUPS_DIR,
            crate::i18n::t("common.default_paren")
        ),
        Some(path) => path.to_string(),
    }
}