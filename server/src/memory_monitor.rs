//! Linux RSS watchdog when debug=true (optional).

use std::sync::Arc;
use std::time::Duration;

use crate::config::Config;

#[cfg(unix)]
use tracing::warn;

const RSS_THRESHOLD_MB: u64 = 100;
const CHECK_INTERVAL_SECS: u64 = 30;

pub fn spawn(config: Arc<Config>) {
    if !config.is_debug() {
        return;
    }

    #[cfg(unix)]
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(CHECK_INTERVAL_SECS)).await;
            log_if_over_threshold();
        }
    });

    #[cfg(not(unix))]
    let _ = config;
}

#[cfg(unix)]
fn log_if_over_threshold() {
    let pid = std::process::id();
    let Some(self_rss_kb) = read_rss_kb(pid) else {
        return;
    };

    let mut children = Vec::new();
    collect_descendants(pid, &mut children);

    let children_rss_kb: u64 = children.iter().map(|(_, _, rss)| rss).sum();
    let total_rss_kb = self_rss_kb.saturating_add(children_rss_kb);
    let threshold_kb = RSS_THRESHOLD_MB * 1024;

    if total_rss_kb <= threshold_kb {
        return;
    }

    let self_name = read_comm(pid).unwrap_or_else(|| "backup-server".to_string());
    let mut parts = vec![format!(
        "{self_name} PID {pid} RSS {:.1} MB",
        kb_to_mb(self_rss_kb)
    )];

    for (cpid, name, rss) in &children {
        parts.push(format!("{name} PID {cpid} RSS {:.1} MB", kb_to_mb(*rss)));
    }

    warn!(
        "Memory {:.1} MB exceeds {} MB threshold: {}",
        kb_to_mb(total_rss_kb),
        RSS_THRESHOLD_MB,
        parts.join("; ")
    );
}

#[cfg(not(unix))]
fn log_if_over_threshold() {
}

#[cfg(unix)]
fn collect_descendants(pid: u32, out: &mut Vec<(u32, String, u64)>) {
    for child_pid in list_child_pids(pid) {
        let rss = read_rss_kb(child_pid).unwrap_or(0);
        let name = read_comm(child_pid).unwrap_or_else(|| "?".to_string());
        out.push((child_pid, name, rss));
        collect_descendants(child_pid, out);
    }
}

#[cfg(unix)]
fn list_child_pids(parent: u32) -> Vec<u32> {
    let mut pids = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return pids;
    };

    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        if read_ppid(pid) == Some(parent) {
            pids.push(pid);
        }
    }
    pids
}

#[cfg(unix)]
fn read_ppid(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rparen = stat.rfind(')')?;
    let rest = stat.get(rparen + 2..)?;
    rest.split_whitespace().next()?.parse().ok()
}

#[cfg(unix)]
fn read_rss_kb(pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    for line in status.lines() {
        if let Some(kb) = line.strip_prefix("VmRSS:") {
            return kb.trim().split_whitespace().next()?.parse().ok();
        }
    }
    None
}

#[cfg(unix)]
fn read_comm(pid: u32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .ok()
        .map(|s| s.trim().to_string())
}

#[cfg(unix)]
fn kb_to_mb(kb: u64) -> f64 {
    kb as f64 / 1024.0
}
