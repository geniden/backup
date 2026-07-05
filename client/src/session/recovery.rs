//! Reconnect and re-submit when the WebSocket drops before a task is settled.

use std::collections::HashSet;
use std::time::Duration;

use once_cell::sync::Lazy;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::i18n;
use crate::protocol;
use crate::session::link::ensure_connected;
use crate::session::pending;

static RECOVERY_ACTIVE: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| Mutex::new(HashSet::new()));

const RECOVERY_CYCLES: u32 = 3;
const RECONNECT_ATTEMPTS: u32 = 6;
const RECONNECT_DELAY: Duration = Duration::from_secs(5);
const SETTLEMENT_WAIT: Duration = Duration::from_secs(90);
const SETTLEMENT_POLL: Duration = Duration::from_secs(2);

/// True while a recovery loop is running for this URL.
pub fn is_active(url: &str) -> bool {
    RECOVERY_ACTIVE
        .try_lock()
        .is_ok_and(|g| g.contains(url))
}

/// Run recovery (also used from link when the pump stops unexpectedly).
pub async fn run_recovery(url: &str, pool: &sqlx::SqlitePool) {
    if !pending::has_any(url).await {
        return;
    }

    {
        let mut active = RECOVERY_ACTIVE.lock().await;
        if !active.insert(url.to_string()) {
            return;
        }
    }

    recovery_body(url, pool).await;
    RECOVERY_ACTIVE.lock().await.remove(url);
}

async fn recovery_body(url: &str, pool: &sqlx::SqlitePool) {
    let names = pending::task_names(url).await.join(", ");
    warn!(
        "{}",
        i18n::t_fmt(
            "session.link_lost",
            &[("url", url), ("tasks", &names)],
        )
    );

    for cycle in 1..=RECOVERY_CYCLES {
        if !pending::has_any(url).await {
            return;
        }

        if !reconnect(url, pool, cycle).await {
            tokio::time::sleep(RECONNECT_DELAY).await;
            continue;
        }

        info!(
            "{}",
            i18n::t_fmt(
                "session.recovery_waiting",
                &[("sec", &SETTLEMENT_WAIT.as_secs().to_string())],
            )
        );

        if wait_for_settlement(url).await {
            info!("{}", i18n::t("session.recovery_restored"));
            return;
        }

        resubmit_pending(url, pool, cycle).await;
    }

    give_up(url).await;
}

async fn reconnect(url: &str, pool: &sqlx::SqlitePool, cycle: u32) -> bool {
    let cycle_str = cycle.to_string();
    let max_str = RECONNECT_ATTEMPTS.to_string();

    for attempt in 1..=RECONNECT_ATTEMPTS {
        let attempt_str = attempt.to_string();
        let pending = pending::snapshot(url).await;
        let Some(work) = pending.first() else {
            return false;
        };

        match ensure_connected(&work.conn, pool).await {
            Ok(_) => {
                info!(
                    "{}",
                    i18n::t_fmt(
                        "session.recovery_reconnected",
                        &[
                            ("slug", &work.conn.slug),
                            ("cycle", &cycle_str),
                            ("attempt", &attempt_str),
                        ],
                    )
                );
                return true;
            }
            Err(e) => {
                warn!(
                    "{}",
                    i18n::t_fmt(
                        "session.recovery_reconnect",
                        &[
                            ("slug", &work.conn.slug),
                            ("cycle", &cycle_str),
                            ("attempt", &attempt_str),
                            ("max", &max_str),
                            ("detail", &e.to_string()),
                        ],
                    )
                );
                tokio::time::sleep(RECONNECT_DELAY).await;
            }
        }
    }

    false
}

async fn wait_for_settlement(url: &str) -> bool {
    let deadline = tokio::time::Instant::now() + SETTLEMENT_WAIT;
    while tokio::time::Instant::now() < deadline {
        if !pending::has_any(url).await {
            return true;
        }
        tokio::time::sleep(SETTLEMENT_POLL).await;
    }
    false
}

async fn resubmit_pending(url: &str, pool: &sqlx::SqlitePool, cycle: u32) {
    let works = pending::snapshot(url).await;
    if works.is_empty() {
        return;
    }

    let cycle_str = cycle.to_string();
    for work in works {
        info!(
            "{}",
            i18n::t_fmt(
                "session.recovery_resubmit",
                &[
                    ("slug", &work.conn.slug),
                    ("name", &work.task.task_name),
                    ("cycle", &cycle_str),
                ],
            )
        );

        let sink = match ensure_connected(&work.conn, pool).await {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    "[{}] Recovery re-submit connect failed for '{}': {}",
                    work.conn.slug, work.task.task_name, e
                );
                continue;
            }
        };

        if let Err(e) = protocol::submit_run_task(&work.conn, &work.task, &sink).await {
            warn!(
                "[{}] Recovery re-submit failed for '{}': {}",
                work.conn.slug, work.task.task_name, e
            );
        }
    }
}

async fn give_up(url: &str) {
    let remaining = pending::clear_url(url).await;
    super::reset_in_flight(url).await;

    for work in remaining {
        error!(
            "{}",
            i18n::t_fmt(
                "session.recovery_gave_up",
                &[
                    ("slug", &work.conn.slug),
                    ("name", &work.task.task_name),
                ],
            )
        );
    }
}
