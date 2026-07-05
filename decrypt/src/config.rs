//! `decrypt.toml` — profiles (slug → password), root path, browser state.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecryptConfig {
    /// Per-connection slug → encrypt_password (same as on backup-server).
    #[serde(default)]
    pub profiles: BTreeMap<String, String>,
    /// Root backups directory (e.g. client `data/backups`).
    pub path: String,
    /// Last folder opened in the browser (saved on exit / navigation).
    #[serde(default)]
    pub last_path: String,
    /// Where to write decrypted plaintext. Empty = next to the `.aes` file.
    #[serde(default)]
    pub output_path: String,
}

impl DecryptConfig {
    pub fn empty() -> Self {
        Self {
            profiles: BTreeMap::new(),
            path: String::new(),
            last_path: String::new(),
            output_path: String::new(),
        }
    }

    pub fn has_any_password(&self) -> bool {
        self.profiles.values().any(|p| !p.is_empty())
    }

    pub fn is_ready(&self) -> bool {
        self.has_any_password() && !self.path.trim().is_empty()
    }

    pub fn profile_slugs(&self) -> Vec<String> {
        self.profiles.keys().cloned().collect()
    }

    pub fn profile_password(&self, slug: &str) -> Option<&str> {
        self.profiles
            .get(slug)
            .filter(|p| !p.is_empty())
            .map(String::as_str)
    }

    pub fn set_profile_password(&mut self, slug: &str, password: String) {
        self.profiles.insert(slug.to_string(), password);
    }

    pub fn remove_profile(&mut self, slug: &str) -> bool {
        self.profiles.remove(slug).is_some()
    }

    pub fn profiles_hint(&self) -> String {
        if self.profiles.is_empty() {
            "(none)".to_string()
        } else {
            let names = self.profile_slugs();
            format!("{} connection name(s): {}", names.len(), names.join(", "))
        }
    }

    pub fn path_hint(&self) -> String {
        if self.path.trim().is_empty() {
            "(not set)".to_string()
        } else {
            self.path.trim().to_string()
        }
    }

    pub fn output_path_hint(&self) -> String {
        if self.output_path.trim().is_empty() {
            "(in-place, next to .aes)".to_string()
        } else {
            self.output_path.trim().to_string()
        }
    }

    /// Auto password: slug from path → profile; else single profile.
    pub fn auto_password_for_file(&self, enc_path: &Path, root: &Path) -> Option<String> {
        if let Some(slug) = slug_from_path(enc_path, root) {
            if let Some(p) = self.profile_password(&slug) {
                return Some(p.to_string());
            }
        }
        if self.profiles.len() == 1 {
            return self.profiles.values().next().cloned();
        }
        None
    }

    pub fn password_for_slug(&self, slug: &str) -> Option<String> {
        self.profile_password(slug).map(str::to_string)
    }
}

/// First path segment under `root` = connection slug (`root/production/foo.aes` → `production`).
pub fn slug_from_path(enc_path: &Path, root: &Path) -> Option<String> {
    let enc = enc_path.canonicalize().ok()?;
    let root = root.canonicalize().ok()?;
    if !enc.starts_with(&root) {
        return None;
    }
    let rel = enc.strip_prefix(&root).ok()?;
    let slug = rel.components().next()?.as_os_str().to_string_lossy();
    if slug.is_empty() {
        return None;
    }
    Some(slug.into_owned())
}

pub fn resolve_config_path(key_file: Option<&Path>) -> PathBuf {
    if let Some(p) = key_file {
        return p.to_path_buf();
    }
    if let Ok(cwd) = std::env::current_dir() {
        let p = cwd.join("decrypt.toml");
        if p.exists() {
            return p;
        }
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("decrypt.toml")))
        .unwrap_or_else(|| PathBuf::from("decrypt.toml"))
}

pub fn load(path: &Path) -> anyhow::Result<DecryptConfig> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Read config: {}", path.display()))?;
    toml::from_str(&content).context("Parse decrypt.toml")
}

pub fn save(path: &Path, cfg: &DecryptConfig) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).ok();
        }
    }
    let body = toml::to_string_pretty(cfg).context("Serialize decrypt.toml")?;
    let content = format!(
        "# backup-decrypt configuration (decrypt.toml)\n\
         # [profiles] — connection name → encrypt_password (one per VPS if keys differ)\n\
         # path       — root folder; inside: {{slug}}/*.aes subfolders\n\
         # last_path  — updated automatically\n\
         # output_path — empty: decrypt next to .aes; or e.g. E:/decrypted\n\n\
         {body}"
    );
    fs::write(path, content).with_context(|| format!("Write {}", path.display()))
}

pub fn canonical_root(cfg: &DecryptConfig) -> anyhow::Result<PathBuf> {
    if cfg.path.trim().is_empty() {
        anyhow::bail!("Backups root folder is not set");
    }
    let p = PathBuf::from(cfg.path.trim());
    if !p.exists() {
        anyhow::bail!("Folder does not exist: {}", p.display());
    }
    p.canonicalize()
        .with_context(|| format!("Canonicalize {}", p.display()))
}

pub fn start_dir(cfg: &DecryptConfig, root: &Path) -> PathBuf {
    if !cfg.last_path.trim().is_empty() {
        let last = PathBuf::from(cfg.last_path.trim());
        if let Ok(can) = last.canonicalize() {
            if can.starts_with(root) {
                return can;
            }
        }
    }
    root.to_path_buf()
}

pub fn is_under_root(path: &Path, root: &Path) -> bool {
    path.canonicalize()
        .ok()
        .zip(root.canonicalize().ok())
        .is_some_and(|(p, r)| p.starts_with(&r))
}

/// Resolve destination path for a decrypted file.
pub fn resolve_plaintext_path(
    enc_path: &Path,
    plain_name: &str,
    cfg: &DecryptConfig,
) -> anyhow::Result<PathBuf> {
    let out_dir = if cfg.output_path.trim().is_empty() {
        enc_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Encrypted file has no parent directory"))?
            .to_path_buf()
    } else {
        let dir = PathBuf::from(cfg.output_path.trim());
        if !dir.exists() {
            fs::create_dir_all(&dir).with_context(|| {
                format!(
                    "Cannot create output_path '{}'. Check that the drive is connected and the path is writable.",
                    dir.display()
                )
            })?;
        }
        dir.canonicalize().with_context(|| {
            format!(
                "output_path '{}' is not available. Check that the drive is connected.",
                cfg.output_path.trim()
            )
        })?
    };

    Ok(out_dir.join(plain_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn slug_from_relative_path() {
        let root = env::temp_dir().join("bd_test_root");
        let slug_dir = root.join("production");
        let enc = slug_dir.join("backup_foo.zip.aes");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&slug_dir).unwrap();
        fs::write(&enc, b"x").unwrap();

        assert_eq!(
            slug_from_path(&enc, &root).as_deref(),
            Some("production")
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn save_load_profiles_roundtrip() {
        let dir = env::temp_dir().join("bd_save_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("decrypt.toml");
        let mut cfg = DecryptConfig::empty();
        cfg.set_profile_password("production", "test-passphrase".into());
        save(&path, &cfg).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(
            loaded.profile_password("production"),
            Some("test-passphrase")
        );
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("[profiles]"));
        assert!(content.contains("production"));
        let _ = fs::remove_dir_all(&dir);
    }
}
