//! AES-256-GCM decrypt — format must match server `backup/encrypt.rs`.

use std::fs;
use std::path::Path;

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{bail, Context};
use sha2::{Digest, Sha256};

pub const MAGIC: &[u8] = b"BACKUPENC1";
const NONCE_LEN: usize = 12;

fn derive_key(password: &str) -> [u8; 32] {
    Sha256::digest(password.as_bytes()).into()
}

/// `backup_foo.zip.aes` → `backup_foo.zip`
pub fn plaintext_filename(encrypted_name: &str) -> Option<String> {
    encrypted_name.strip_suffix(".aes").map(str::to_string)
}

pub fn decrypt_bytes(data: &[u8], password: &str) -> anyhow::Result<Vec<u8>> {
    if data.len() < MAGIC.len() + NONCE_LEN + 16 {
        bail!("File too short for BACKUPENC1 format");
    }
    if &data[..MAGIC.len()] != MAGIC {
        bail!("Not a BACKUPENC1 file (bad magic header)");
    }

    let nonce = Nonce::from_slice(&data[MAGIC.len()..MAGIC.len() + NONCE_LEN]);
    let ciphertext = &data[MAGIC.len() + NONCE_LEN..];

    let key = derive_key(password);
    let cipher = Aes256Gcm::new_from_slice(&key).context("AES key init")?;
    let plain = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| anyhow::anyhow!("Decryption failed (wrong password or corrupted file)"))?;
    Ok(plain)
}

pub fn is_wrong_password(err: &anyhow::Error) -> bool {
    err.to_string().contains("wrong password")
}

pub fn decrypt_file(enc_path: &Path, out_path: &Path, password: &str) -> anyhow::Result<u64> {
    let data = fs::read(enc_path)
        .with_context(|| format!("Read {}", enc_path.display()))?;
    let plain = decrypt_bytes(&data, password)?;
    fs::write(out_path, &plain).with_context(|| format!("Write {}", out_path.display()))?;
    Ok(plain.len() as u64)
}
