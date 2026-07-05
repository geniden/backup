//! Cron scheduler with connect-on-demand WebSocket sessions.

use std::time::Duration;

use chrono::Local;
use tokio_cron_scheduler::{Job, JobScheduler};
use tracing::{error, info, warn};

use crate::cron;
use crate::db;
use crate::i18n;
use crate::models::connection::Connection;
use crate::models::task::Task;
use crate::protocol;
use crate::session;

pub async fn start_scheduler() -> ! {
    let pool = match db::init_db().await {
        Ok(p) => p,
        Err(e) => {
            error!("Database init failed: {}", e);
            std::process::exit(1);
        }
    };

    let connections = match db::list_enabled_connections(&pool).await {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to load connections: {}", e);
            std::process::exit(1);
        }
    };

    let active: Vec<_> = {
        let mut out = Vec::new();
        for conn in connections {
            if db::connection_has_enabled_tasks(&pool, &conn.id).await {
                out.push(conn);
            }
        }
        out
    };

    info!(
        "Starting scheduler with {} active connection(s)",
        active.len()
    );

    let mut cron_scheduler = JobScheduler::new()
        .await
        .expect("Failed to create cron scheduler");

    for conn in &active {
        session::warm_if_needed(conn, &pool).await;

        let tasks = match db::list_tasks_for_connection(&pool, &conn.id).await {
            Ok(t) => t,
            Err(e) => {
                error!("[{}] Failed to load tasks: {}", conn.slug, e);
                continue;
            }
        };

        for task in tasks.into_iter().filter(|t| t.enabled) {
            register_cron_job(&mut cron_scheduler, pool.clone(), conn.clone(), task).await;
        }
    }

    cron_scheduler
        .start()
        .await
        .expect("Failed to start cron scheduler");

    info!("Scheduler running. Press Ctrl+C to stop.");

    loop {
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}

async fn register_cron_job(
    scheduler: &mut JobScheduler,
    pool: sqlx::SqlitePool,
    conn: Connection,
    task: Task,
) {
    let cron_expr = match cron::to_six_field(&cron::normalize_schedule(&task.schedule)) {
        Ok(expr) => expr,
        Err(e) => {
            error!(
                "[{}] Invalid schedule for task '{}': {}",
                conn.slug, task.task_name, e
            );
            return;
        }
    };

    let conn_slug = conn.slug.clone();
    let task_name = task.task_name.clone();
    let task_schedule = task.schedule.clone();

    let job = match Job::new_async_tz(cron_expr.as_str(), Local, move |_uuid, _lock| {
        let pool = pool.clone();
        let conn = conn.clone();
        let task = task.clone();
        Box::pin(async move {
            if let Err(e) = run_scheduled_task(&pool, &conn, &task).await {
                warn!(
                    "[{}] Scheduled task '{}' failed: {}",
                    conn.slug, task.task_name, e
                );
            }
        })
    }) {
        Ok(j) => j,
        Err(e) => {
            error!(
                "[{}] Failed to register cron job for '{}': {}",
                conn_slug, task_name, e
            );
            return;
        }
    };

    if let Err(e) = scheduler.add(job).await {
        error!(
            "[{}] Failed to add cron job for '{}': {}",
            conn_slug, task_name, e
        );
    } else {
        info!(
            "[{}] Registered task '{}' ({}, cron: {})",
            conn_slug,
            task_name,
            cron::normalize_schedule(&task_schedule),
            cron_expr
        );
    }
}

async fn run_scheduled_task(
    pool: &sqlx::SqlitePool,
    conn: &Connection,
    task: &Task,
) -> anyhow::Result<()> {
    info!(
        "{}",
        i18n::t_fmt(
            "scheduler.running_task",
            &[("slug", &conn.slug), ("name", &task.task_name)],
        )
    );

    const SUBMIT_MAX_ATTEMPTS: u32 = 3;
    const SUBMIT_RETRY_DELAY: Duration = Duration::from_secs(60);
    let max_str = SUBMIT_MAX_ATTEMPTS.to_string();
    let sec_str = SUBMIT_RETRY_DELAY.as_secs().to_string();

    for attempt in 1..=SUBMIT_MAX_ATTEMPTS {
        let attempt_str = attempt.to_string();
        let sink = match session::ensure_connected(conn, pool).await {
            Ok(s) => s,
            Err(e) => {
                let detail = e.to_string();
                if attempt < SUBMIT_MAX_ATTEMPTS {
                    info!(
                        "{}",
                        i18n::t_fmt(
                            "scheduler.retry",
                            &[
                                ("slug", &conn.slug),
                                ("name", &task.task_name),
                                ("attempt", &attempt_str),
                                ("max", &max_str),
                                ("detail", &detail),
                                ("sec", &sec_str),
                            ],
                        )
                    );
                    tokio::time::sleep(SUBMIT_RETRY_DELAY).await;
                    continue;
                } else {
                    error!(
                        "{}",
                        i18n::t_fmt(
                            "scheduler.skipped",
                            &[
                                ("slug", &conn.slug),
                                ("name", &task.task_name),
                                ("max", &max_str),
                                ("detail", &detail),
                            ],
                        )
                    );
                    return Ok(());
                }
            }
        };

        match protocol::submit_run_task(conn, task, &sink).await {
            Ok(()) => {
                session::register_pending(conn.clone(), task.clone()).await;
                info!(
                    "{}",
                    i18n::t_fmt(
                        "scheduler.accepted",
                        &[("slug", &conn.slug), ("name", &task.task_name)],
                    )
                );
                return Ok(());
            }
            Err(e) => {
                let detail = e.to_string();
                if attempt < SUBMIT_MAX_ATTEMPTS {
                    info!(
                        "{}",
                        i18n::t_fmt(
                            "scheduler.retry",
                            &[
                                ("slug", &conn.slug),
                                ("name", &task.task_name),
                                ("attempt", &attempt_str),
                                ("max", &max_str),
                                ("detail", &detail),
                                ("sec", &sec_str),
                            ],
                        )
                    );
                    tokio::time::sleep(SUBMIT_RETRY_DELAY).await;
                } else {
                    error!(
                        "{}",
                        i18n::t_fmt(
                            "scheduler.skipped",
                            &[
                                ("slug", &conn.slug),
                                ("name", &task.task_name),
                                ("max", &max_str),
                                ("detail", &detail),
                            ],
                        )
                    );
                    return Ok(());
                }
            }
        }
    }

    Ok(())
}
