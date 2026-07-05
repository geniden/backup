//! Run whitelisted scripts from scripts_dir.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf, Component};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tracing::{info, warn};

use super::TaskResult;
use super::naming;
use crate::paths;
use crate::utils::hash_file_sha256;

pub struct ShellTask {
    pub task_name: String,
    pub script_name: String,
    pub script_args: Vec<String>,
    pub timeout_secs: u64,
    pub files_dir: String,
    pub scripts_dir: String,
}

impl ShellTask {
    pub async fn execute(&self) -> Result<TaskResult> {
        if !Self::is_safe_script_name(&self.script_name) {
            warn!("🚫 Blocked path traversal attempt in script name: {}", self.script_name);
            return Err(anyhow::anyhow!("Invalid script name"));
        }

        let script_path = PathBuf::from(&self.scripts_dir).join(&self.script_name);
        
        let scripts_dir_canonical = std::fs::canonicalize(&self.scripts_dir)
            .map_err(|_| anyhow::anyhow!("Scripts directory not found: {}", self.scripts_dir))?;
        
        let script_path_canonical = std::fs::canonicalize(&script_path)
            .map_err(|_| {
                warn!("🚫 Script not found: {}", script_path.display());
                anyhow::anyhow!("Script not found: {}", self.script_name)
            })?;
        
        if !script_path_canonical.starts_with(&scripts_dir_canonical) {
            warn!("🚫 Blocked path escape attempt: {:?}", script_path_canonical);
            return Err(anyhow::anyhow!("Access denied"));
        }

        if !script_path_canonical.is_file() {
            return Err(anyhow::anyhow!("Not a file: {}", self.script_name));
        }

        info!("🔒 Running authorized script: {} {:?}", self.script_name, self.script_args);
        
        let filename = naming::backup_filename(&self.task_name, "shell", "txt");
        let filepath = paths::join(&self.files_dir, &filename);
        let filepath_str = filepath.to_string_lossy().into_owned();
        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.arg("/C");
            
            let mut full_command = format!("\"{}\"", script_path_canonical.display());
            for arg in &self.script_args {
                full_command.push(' ');
                full_command.push_str(arg);
            }
            c.arg(&full_command);
            c
        } else {
            let mut c = Command::new("bash");
            c.arg(&script_path_canonical);
            for arg in &self.script_args {
                c.arg(arg);
            }
            c
        };
        
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let start_time = std::time::Instant::now();
        
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(self.timeout_secs),
            cmd.output()
        )
        .await;

        let elapsed = start_time.elapsed();

        match result {
            Ok(Ok(output)) => {
                let stdout = Self::decode_output(&output.stdout);
                let stderr = Self::decode_output(&output.stderr);
                                
                let mut file = tokio::fs::File::create(&filepath).await
                    .context("Failed to create output file")?;
                
                let result_content = format!(
                    "=== Script Output ===\n\
                     Script: {}\n\
                     Args: {:?}\n\
                     Exit Code: {:?}\n\
                     Duration: {:.2}s\n\
                     \n\
                     === STDOUT ===\n\
                     {}\n\
                     \n\
                     === STDERR ===\n\
                     {}\n",
                    self.script_name,
                    self.script_args,
                    output.status.code(),
                    elapsed.as_secs_f64(),
                    stdout,
                    stderr
                );
                
                file.write_all(result_content.as_bytes()).await?;
                file.sync_all().await?;

                let size_bytes = result_content.len() as u64;

                let file_hash = hash_file_sha256(&filepath_str).unwrap_or_else(|e| {
                    warn!("Could not hash '{}': {}", filepath_str, e);
                    "unknown".to_string()
                });

                if output.status.success() {
                    info!("Script completed in {:.2}s", elapsed.as_secs_f64());
                } else {
                    warn!("Script failed with exit code: {:?}", output.status.code());
                }

                Ok(TaskResult {
                    filename,
                    size_bytes: size_bytes as i64,
                    files_count: 1,
                    file_hash,
                })
            }
            
            Ok(Err(e)) => {
                Err(anyhow::anyhow!("Failed to execute script: {}", e))
            }
            
            Err(_) => {
                Err(anyhow::anyhow!(
                    "Script timed out after {} seconds",
                    self.timeout_secs
                ))
            }
        }
    }

    fn is_safe_script_name(name: &str) -> bool {
        if name.is_empty() {
            return false;
        }
        
        let dangerous_patterns = ["..", "\\", "//", "\\\\", "%2e%2e", "%2f", "%5c"];
        let lower = name.to_lowercase();
        for pattern in dangerous_patterns {
            if lower.contains(pattern) {
                return false;
            }
        }
        
        let path = Path::new(name);
        for component in path.components() {
            match component {
                Component::Normal(_) => {} // OK
                _ => return false,
            }
        }
        
        true
    }

    fn decode_output(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }
}