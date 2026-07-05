//! AES-256-GCM encryption over completed backup files (.zip / .txt → *.aes).

use std::path::Path;
use std::time::Instant;

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::Context;
use rand::RngCore;
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::config::Config;
use crate::utils::hash_file_sha256;

use super::TaskResult;

pub const MAGIC: &[u8] = b"BACKUPENC1";
const NONCE_LEN: usize = 12;

/// `backup_foo.zip` → `backup_foo.zip.aes`
pub fn encrypted_filename(plain: &str) -> String {
    format!("{plain}.aes")
}

fn derive_key(password: &str) -> [u8; 32] {
    Sha256::digest(password.as_bytes()).into()
}

fn encrypt_bytes(plain: &[u8], password: &str) -> anyhow::Result<Vec<u8>> {
    let key = derive_key(password);
    let cipher = Aes256Gcm::new_from_slice(&key).context("AES key init")?;

    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plain)
        .map_err(|e| anyhow::anyhow!("AES encrypt failed: {e}"))?;

    let mut out = Vec::with_capacity(MAGIC.len() + NONCE_LEN + ciphertext.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Encrypts to `filename.aes`, deletes plaintext, updates hash/size.
/// Caller must check per-task encrypt flag and that `encrypt_password` is set.
pub async fn maybe_encrypt_backup_file(
    config: &Config,
    files_dir: &Path,
    result: TaskResult,
) -> anyhow::Result<TaskResult> {
    if config.encrypt_password.is_empty() {
        anyhow::bail!("encrypt_password is empty");
    }

    let plain_name = &result.filename;
    let plain_path = files_dir.join(plain_name);
    let enc_name = encrypted_filename(plain_name);
    let enc_path = files_dir.join(&enc_name);

    let password = config.encrypt_password.clone();
    let started = Instant::now();

    let (enc_size, file_hash) = tokio::task::spawn_blocking(move || {
        let plain = std::fs::read(&plain_path)
            .with_context(|| format!("Read plaintext backup: {}", plain_path.display()))?;
        let encrypted = encrypt_bytes(&plain, &password)?;
        std::fs::write(&enc_path, &encrypted)
            .with_context(|| format!("Write encrypted backup: {}", enc_path.display()))?;
        std::fs::remove_file(&plain_path)
            .with_context(|| format!("Remove plaintext backup: {}", plain_path.display()))?;
        let hash = hash_file_sha256(enc_path.to_string_lossy().as_ref())
            .unwrap_or_else(|e| {
                warn!("Could not hash '{}': {}", enc_path.display(), e);
                "unknown".to_string()
            });
        Ok::<_, anyhow::Error>((encrypted.len() as u64, hash))
    })
    .await
    .context("encrypt task join")??;

    let elapsed = started.elapsed();
    if config.is_debug() {
        info!(
            "AES encrypt {plain_name} → {enc_name} in {:.2}s ({} → {} bytes)",
            elapsed.as_secs_f64(),
            result.size_bytes,
            enc_size
        );
    } else {
        info!(
            "Encrypted {plain_name} → {enc_name} ({enc_size} bytes, {:.2}s)",
            elapsed.as_secs_f64()
        );
    }

    Ok(TaskResult {
        filename: enc_name,
        size_bytes: enc_size as i64,
        files_count: result.files_count,
        file_hash,
    })
}
