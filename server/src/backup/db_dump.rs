//! MySQL, PostgreSQL, SQLite dump and zip.

use anyhow::{Context, Result};
use tokio::fs as tokio_fs;
use tokio::io::AsyncWriteExt;
use tokio::task::spawn_blocking;
use tracing::info;
use tracing::warn;
use crate::paths;
use crate::utils::hash_file_sha256;

use super::naming;
use super::TaskResult;

use std::io::Write;

pub async fn dump_sqlite(
    _task_id: &str,
    task_name: &str,
    task_type: &str,
    data: &serde_json::Value,
) -> Result<TaskResult> {
    let db_path = data["db_path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("db_path required for SQLite"))?;

    let db_filename = naming::backup_filename(task_name, task_type, "db");
    let zip_filename = naming::archive_zip_name(&db_filename);

    let files_dir = data.get("files_dir").and_then(|v| v.as_str()).unwrap_or("./data/temp");
    let db_path_dest = paths::join(files_dir, &db_filename);
    let zip_path_dest = paths::join(files_dir, &zip_filename);
    let zip_path_for_hash = zip_path_dest.to_string_lossy().into_owned();

    tracing::debug!("SQLite copy: {} -> {}", db_path, paths::display_path(&db_path_dest));
    tokio_fs::copy(db_path, &db_path_dest).await.context("Failed to copy DB")?;

    let original_meta = tokio_fs::metadata(&db_path).await?;
    let copied_meta = tokio_fs::metadata(&db_path_dest).await?;

    if original_meta.len() != copied_meta.len() {
        return Err(anyhow::anyhow!(
            "SQLite copy size mismatch: original {} bytes, but copied {} bytes",
            original_meta.len(),
            copied_meta.len()
        ));
    }

    let zip_size = spawn_blocking(move || {
        let file = std::fs::File::create(&zip_path_dest)?;
        let mut zip = zip::ZipWriter::new(file);

        let options = zip::write::FileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        zip.start_file(&db_filename, options)?;
        zip.write_all(&std::fs::read(&db_path_dest)?)?;
        zip.finish()?;

        let db_temp_path = db_path_dest.clone();
        if let Err(e) = std::fs::remove_file(&db_temp_path) {
            warn!("Could not delete temp SQLite file {}: {}", paths::display_path(&db_temp_path), e);
        }

        Ok::<u64, std::io::Error>(std::fs::metadata(&zip_path_dest)?.len())
    })
    .await
    .map_err(|e| anyhow::anyhow!("Archive error: {}", e))??;

    tracing::debug!("SQLite archived: {} ({} bytes)", zip_filename, copied_meta.len());

    let file_hash = hash_file_sha256(&zip_path_for_hash)
        .unwrap_or_else(|e| {
            warn!("Could not hash '{}': {}", zip_path_for_hash, e);
            "unknown".to_string()
        });

    Ok(TaskResult {
        filename: zip_filename,
        size_bytes: zip_size as i64,
        files_count: 1,
        file_hash,
    })
}

pub async fn dump_mysql(_task_id: &str, task_name: &str, data: &serde_json::Value, mysqldump_path: &str) -> Result<TaskResult> {
    if crate::utils::is_explicit_tool_path(mysqldump_path) {
        let p = std::path::Path::new(mysqldump_path);
        if !p.exists() {
            return Err(anyhow::anyhow!(
                "mysqldump not found at configured path '{}'. Configure correct path in config.toml",
                mysqldump_path
            ));
        }
    } else if !crate::utils::command_available(mysqldump_path) {
        return Err(anyhow::anyhow!(
            "Command '{}' not found in PATH. Configure correct path in config.toml",
            mysqldump_path
        ));
    }

    let db_host = data["db_host"].as_str().unwrap_or("127.0.0.1");
    let db_port = data["db_port"].as_u64().map(|v| v as u16).unwrap_or(3306);
    let db_user = data["db_user"].as_str().ok_or_else(|| anyhow::anyhow!("`db_user` required for MySQL"))?;
    let db_pass = data["db_pass"].as_str().unwrap_or("");
    let db_name = data["db_name"].as_str().ok_or_else(|| anyhow::anyhow!("`db_name` required for MySQL"))?;

    let filename = naming::backup_filename(task_name, "mysql_dump", "sql");
    let files_dir = data
        .get("files_dir")
        .and_then(|v| v.as_str())
        .unwrap_or("./data/temp");
    let dest_path = paths::join(files_dir, &filename);

    tracing::debug!("mysqldump: {}@{}:{}/{}", db_user, db_host, db_port, db_name);

    let output = tokio::process::Command::new(mysqldump_path)
        .arg("-h").arg(db_host)
        .arg("-P").arg(&db_port.to_string())
        .arg("-u").arg(db_user)
        .arg("--single-transaction")
        .arg("--quick")
        .arg("--default-character-set=utf8mb4")
        .arg(&format!("-p{}", db_pass))
        .arg(db_name)
        .output()
        .await
        .context("Failed to run mysqldump")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("mysqldump failed: {}", stderr));
    }

    let mut file = tokio_fs::File::create(&dest_path).await?;
    file.write_all(&output.stdout).await?;
    file.sync_all().await?;
    drop(file); // ensures buffer flush to disk

    let written_metadata = tokio_fs::metadata(&dest_path).await?;
    let actual_size = written_metadata.len();
    let expected_size = output.stdout.len() as u64;

    if actual_size != expected_size {
        return Err(anyhow::anyhow!(
            "MySQL dump size mismatch: expected {} bytes, but wrote {} bytes (disk full or I/O error)",
            expected_size, actual_size
        ));
    }

    tracing::debug!("MySQL dump: {} bytes", actual_size);

    let file_hash = hash_file_sha256(&dest_path.to_string_lossy()).unwrap_or_else(|e| {
        warn!("Could not hash '{}': {}", paths::display_path(&dest_path), e);
        "unknown".to_string()
    });

    Ok(TaskResult {
        filename,
        size_bytes: actual_size as i64,
        files_count: 1,
        file_hash,
    })
}

pub async fn dump_postgresql(_task_id: &str, task_name: &str, data: &serde_json::Value, pg_dump_path: &str) -> Result<TaskResult> {
    if crate::utils::is_explicit_tool_path(pg_dump_path) {
        let p = std::path::Path::new(pg_dump_path);
        if !p.exists() {
            return Err(anyhow::anyhow!(
                "pg_dump not found at configured path '{}'. Configure correct path in config.toml",
                pg_dump_path
            ));
        }
    } else if !crate::utils::command_available(pg_dump_path) {
        return Err(anyhow::anyhow!(
            "Command '{}' not found in PATH. Configure correct path in config.toml",
            pg_dump_path
        ));
    }

    let db_host = data["db_host"].as_str().unwrap_or("127.0.0.1");
    let db_port = data["db_port"].as_u64().map(|v| v as u16).unwrap_or(5432);
    let db_user = data["db_user"].as_str().ok_or_else(|| anyhow::anyhow!("`db_user` required for PostgreSQL"))?;
    let db_pass = data["db_pass"].as_str().unwrap_or("");
    let db_name = data["db_name"].as_str().ok_or_else(|| anyhow::anyhow!("`db_name` required for PostgreSQL"))?;

    let filename = naming::backup_filename(task_name, "postgresql_dump", "sql");
    let files_dir = data
        .get("files_dir")
        .and_then(|v| v.as_str())
        .unwrap_or("./data/temp");
    let dest_path = paths::join(files_dir, &filename);

    tracing::debug!("pg_dump: {}@{}:{}/{}", db_user, db_host, db_port, db_name);

    let output = tokio::process::Command::new(pg_dump_path)
        .arg("-h").arg(db_host)
        .arg("-p").arg(&db_port.to_string())
        .arg("-U").arg(db_user)
        .arg(db_name)
        .env("PGPASSWORD", db_pass)
        .output()
        .await
        .context("Failed to run pg_dump")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("pg_dump failed: {}", stderr));
    }

    let mut file = tokio_fs::File::create(&dest_path).await?;
    file.write_all(&output.stdout).await?;
    file.sync_all().await?;
    drop(file); // ensures buffer flush to disk

    let written_metadata = tokio_fs::metadata(&dest_path).await?;
    let actual_size = written_metadata.len();
    let expected_size = output.stdout.len() as u64;

    if actual_size != expected_size {
        return Err(anyhow::anyhow!(
            "PostgreSQL dump size mismatch: expected {} bytes, but wrote {} bytes (disk full or I/O error)",
            expected_size, actual_size
        ));
    }

    tracing::debug!("PostgreSQL dump: {} bytes", actual_size);

    let file_hash = hash_file_sha256(&dest_path.to_string_lossy()).unwrap_or_else(|e| {
        warn!("Could not hash '{}': {}", paths::display_path(&dest_path), e);
        "unknown".to_string()
    });

    Ok(TaskResult {
        filename,
        size_bytes: actual_size as i64,
        files_count: 1,
        file_hash,
    })
}

pub async fn dump(
    task_id: &str,
    task_name: &str,
    task_type: &str,
    data: &serde_json::Value,
    mysqldump_path: &str,
    pg_dump_path: &str,
    _sqlite3_path: &str,
) -> Result<TaskResult> {
    let provider = data["provider"].as_str().unwrap_or_else(|| match task_type {
        "postgresql_dump" => "postgresql",
        "sqlite_dump" => "sqlite",
        _ => "mysql",
    });

    match provider {
        "sqlite" | "sqlite3" => dump_sqlite(task_id, task_name, task_type, data).await,
        "mysql" | "mariadb" => dump_mysql(task_id, task_name, data, mysqldump_path).await,
        "postgresql" | "postgres" => dump_postgresql(task_id, task_name, data, pg_dump_path).await,
        _ => Err(anyhow::anyhow!("Unsupported provider: {}", provider)),
    }
}

pub async fn dump_and_archive(
    task_id: &str,
    task_name: &str,
    task_type: &str,
    data: &serde_json::Value,
    mysqldump_path: &str,
    pg_dump_path: &str,
) -> Result<TaskResult> {
    tracing::debug!("[{}] sql dump + zip start", task_id);

    let result = dump(
        task_id,
        task_name,
        task_type,
        data,
        mysqldump_path,
        pg_dump_path,
        "",
    )
    .await?;
    let sql_filename = result.filename.clone();

    let files_dir = data.get("files_dir").and_then(|v| v.as_str()).unwrap_or("./data/temp");
    let sql_path = paths::join(files_dir, &sql_filename);

    if result.size_bytes == 0 {
        return Err(anyhow::anyhow!("SQL dump is empty: {}", paths::display_path(&sql_path)));
    }

    info!("[{}] SQL size: {} bytes", task_id, result.size_bytes);

    if result.size_bytes >= 200_000_000 {
        info!("[{}] streaming zip (>200 MB)", task_id);
        dump_and_archive_streaming(task_id, data, result).await
    } else {
        dump_and_archive_small(task_id, data, result).await
    }
}

async fn dump_and_archive_small(
    task_id: &str,
    data: &serde_json::Value,
    result: TaskResult,
) -> Result<TaskResult> {
    let sql_filename = result.filename.clone();
    let files_dir = data.get("files_dir").and_then(|v| v.as_str()).unwrap_or("./data/temp");
    let sql_path = paths::join(files_dir, &sql_filename);

    tracing::debug!("[{}] zip sql ({} bytes)", task_id, result.size_bytes);

    let content = tokio::fs::read(&sql_path).await
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", paths::display_path(&sql_path), e))?;

    if content.is_empty() {
        return Err(anyhow::anyhow!(
            "Read SQL file is empty — check database access: {}",
            paths::display_path(&sql_path)
        ));
    }

    let zip_filename = naming::archive_zip_name(&sql_filename);
    let zip_path = paths::join(files_dir, &zip_filename);
    let zip_path_for_hash = zip_path.clone();

    let actual_zip_size = spawn_blocking(move || {
        let file = std::fs::File::create(&zip_path)?;
        let mut zip = zip::ZipWriter::new(file);

        let options = zip::write::FileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        zip.start_file(&sql_filename, options)?;
        zip.write_all(&content)?;
        zip.finish()?;

        if let Err(e) = std::fs::remove_file(&sql_path) {
            warn!("Could not delete temp SQL {}: {}", paths::display_path(&sql_path), e);
        }

        Ok::<u64, std::io::Error>(std::fs::metadata(&zip_path)?.len())
    })
    .await
    .map_err(|e| anyhow::anyhow!("Archive error: {}", e))??;

    let file_hash = hash_file_sha256(&zip_path_for_hash.to_string_lossy()).unwrap_or_else(|e| {
        warn!("[{}] hash failed: {}", task_id, e);
        "unknown".to_string()
    });

    Ok(TaskResult {
        filename: zip_filename,
        size_bytes: actual_zip_size as i64,
        files_count: 1,
        file_hash,
    })
}

async fn dump_and_archive_streaming(
    task_id: &str,
    data: &serde_json::Value,
    result: TaskResult,
) -> Result<TaskResult> {
    tracing::debug!("[{}] streaming archive", task_id);

    let sql_filename = result.filename.clone();
    let files_dir = data.get("files_dir").and_then(|v| v.as_str()).unwrap_or("./data/temp");
    let sql_path = paths::join(files_dir, &sql_filename);

    info!("[{}] streaming zip: {}", task_id, &sql_filename);

    let zip_filename = naming::archive_zip_name(&sql_filename);
    let zip_path = paths::join(files_dir, &zip_filename);
    let zip_path_for_hash = zip_path.clone();

    let actual_zip_size = spawn_blocking(move || {
        let mut input = std::fs::File::open(&sql_path)?;
        let output = std::fs::File::create(&zip_path)?;
        let mut zip = zip::ZipWriter::new(output);

        let options = zip::write::FileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        zip.start_file(&sql_filename, options)?;
        std::io::copy(&mut input, &mut zip)?;
        zip.finish()?;

        if let Err(e) = std::fs::remove_file(&sql_path) {
            warn!("Could not delete temp SQL file {}: {}", paths::display_path(&sql_path), e);
        } else {
            info!("Deleted temp SQL file: {}", paths::display_path(&sql_path));
        }
        
        Ok::<u64, std::io::Error>(std::fs::metadata(&zip_path)?.len())
    })
    .await
    .map_err(|e| anyhow::anyhow!("Archive error: {}", e))??;

    info!(
        "[{}] streaming done: {} ({} -> {} bytes)",
        task_id, zip_filename, result.size_bytes, actual_zip_size
    );

    let file_hash = hash_file_sha256(&zip_path_for_hash.to_string_lossy())
        .unwrap_or_else(|e| {
            warn!(
                "[{}] Could not hash '{}': {}",
                task_id,
                paths::display_path(&zip_path_for_hash),
                e
            );
            "unknown".to_string()
        });

    Ok(TaskResult {
        filename: zip_filename,
        size_bytes: actual_zip_size as i64,
        files_count: 1,
        file_hash,
    })
}