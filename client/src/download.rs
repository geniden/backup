//! HTTPS download of backup files with SHA256 verification.

use std::path::PathBuf;
use std::time::Duration;

use reqwest::header::HeaderValue;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use tracing::{info, warn};

use sqlx::SqlitePool;

use crate::db;
use crate::models::connection::Connection;
use crate::paths;

pub async fn download_backup(
    pool: &SqlitePool,
    conn: &Connection,
    url: &str,
    expected_hash: &str,
) -> anyhow::Result<(PathBuf, u64)> {
    let filename = crate::protocol::filename_from_download_url(url)
        .ok_or_else(|| anyhow::anyhow!("Invalid download URL: {}", url))?;

    let backups_root = db::backups_root_path(pool).await?;
    let dest_dir = paths::connection_backups_dir_with(&backups_root, &conn.slug);
    tokio::fs::create_dir_all(&dest_dir).await?;
    let final_path = dest_dir.join(&filename);

    let url = crate::tls::normalize_download_url(conn, url);
    let client = crate::tls::build_http_client(conn)?;

    let device_id = crate::device_id::compute_device_id()?;

    for attempt in 1..=3 {
        info!(
            "[{}] Download {}/3: {}",
            conn.slug, attempt, filename
        );

        let resp = client
            .get(&url)
            .header("X-Device-Id", HeaderValue::from_str(&device_id)?)
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("Download failed (HTTP {})", resp.status());
        }

        tokio::fs::write(&final_path, resp.bytes().await?).await?;

        if expected_hash.is_empty() || expected_hash == "unknown" {
            let size = file_size(&final_path).await?;
            info!(
                "[{}] Saved: {} ({})",
                conn.slug,
                final_path.display(),
                crate::format::format_bytes(size)
            );
            return Ok((final_path, size));
        }

        let actual = compute_sha256(&final_path).await?;
        if actual == expected_hash {
            let size = file_size(&final_path).await?;
            info!(
                "[{}] Saved: {} ({})",
                conn.slug,
                final_path.display(),
                crate::format::format_bytes(size)
            );
            return Ok((final_path, size));
        }

        let bad = dest_dir.join(format!("{}_bad{}", filename, attempt));
        tokio::fs::rename(&final_path, &bad).await?;
        warn!("[{}] Hash mismatch on attempt {}", conn.slug, attempt);

        if attempt < 3 {
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    anyhow::bail!("Hash verification failed")
}

async fn file_size(filepath: &PathBuf) -> anyhow::Result<u64> {
    let meta = tokio::fs::metadata(filepath).await?;
    Ok(meta.len())
}

async fn compute_sha256(filepath: &PathBuf) -> anyhow::Result<String> {
    let mut file = tokio::fs::File::open(filepath).await?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}
