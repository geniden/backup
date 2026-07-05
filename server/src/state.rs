//! Shared app state: config, WS client, registry, queue.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use sqlx::SqlitePool;
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::config::Config;
use crate::run_queue::RunQueue;
use crate::task_registry::TaskRegistry;

/// Result of device_id check on WebSocket auth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceAuthOutcome {
    /// First client — device_id written to config.toml.
    Registered,
    /// device_id matches config.toml.
    Verified,
}

fn device_id_short(id: &str) -> String {
    if id.len() >= 64 {
        format!("{}...{}", &id[..8], &id[56..])
    } else if id.len() > 16 {
        format!("{}...", &id[..16])
    } else {
        id.to_string()
    }
}

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub config_path: Arc<PathBuf>,
    pub device_id: Arc<Mutex<String>>,
    pub db: SqlitePool,
    pub client_channels: Arc<Mutex<HashMap<String, tokio::sync::mpsc::Sender<String>>>>,
    pub is_connected: Arc<Mutex<bool>>,
    /// Signals the active WebSocket handler to exit so a reconnect can take over.
    pub ws_stop: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    pub busy_connections: Arc<Mutex<HashSet<String>>>,
    pub task_registry: Arc<TaskRegistry>,
    pub run_queue: Arc<RunQueue>,
}

impl AppState {
    /// First connect registers device_id; later connects must match.
    pub async fn verify_or_register_device(
        &self,
        offered: &str,
    ) -> anyhow::Result<DeviceAuthOutcome> {
        let offered = offered.trim();
        if offered.is_empty() {
            anyhow::bail!("Missing device_id");
        }
        if offered.len() != 64 || !offered.chars().all(|c| c.is_ascii_hexdigit()) {
            anyhow::bail!("Invalid device_id format (expected 64 hex chars, got {})", offered.len());
        }

        let short = device_id_short(offered);
        let mut stored = self.device_id.lock().await;
        if stored.is_empty() {
            *stored = offered.to_string();
            let mut cfg = (*self.config).clone();
            cfg.device_id = offered.to_string();
            cfg.save(self.config_path.as_path())?;
            info!(
                "Device authorized: registered new client ({short}) — saved to config.toml"
            );
            return Ok(DeviceAuthOutcome::Registered);
        }

        if *stored == offered {
            info!("Device authorized: client verified ({short})");
            return Ok(DeviceAuthOutcome::Verified);
        }

        warn!(
            "Device REJECTED: mismatch (offered {short}, expected {})",
            device_id_short(&stored)
        );
        anyhow::bail!("Device not authorized")
    }

    pub async fn device_id_matches(&self, offered: Option<&str>) -> bool {
        let Some(offered) = offered.map(str::trim).filter(|s| !s.is_empty()) else {
            return false;
        };
        let stored = self.device_id.lock().await;
        !stored.is_empty() && *stored == offered
    }
}
