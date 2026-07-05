//! WebSocket pump: connect, auth, sync, inbound message loop.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use futures::StreamExt;
use tokio::sync::{watch, Mutex};
use tracing::{debug, warn};

use crate::i18n;
use crate::manager::{ConnectionManager, SharedSplitSink};
use crate::models::connection::Connection;
use crate::protocol;
use crate::ws_inbound::InboundPump;

use super::pending;
use super::recovery;
use super::{UrlSessionInner, DISCONNECT_GRACE, SESSION_HOLD, SESSION_HOLD_MINUTES};

pub async fn establish(
    conn: Connection,
    pool: sqlx::SqlitePool,
    manager: &'static ConnectionManager,
) -> anyhow::Result<Arc<UrlSessionInner>> {
    debug!(
        "{}",
        i18n::t_fmt(
            "session.connecting",
            &[("slug", &conn.slug), ("url", &conn.url)],
        )
    );

    let (ws_stream, _) = crate::tls::connect_ws(&conn).await?;
    let (write, mut read) = ws_stream.split();
    let write_arc = Arc::new(tokio::sync::Mutex::new(write));

    protocol::authenticate_and_sync(&conn, &pool, &mut read, &write_arc).await?;

    let (stop_tx, stop_rx) = watch::channel(false);
    let url = conn.url.clone();
    let conn_for_pump = conn.clone();
    let pool_for_pump = pool.clone();
    let write_for_pump = Arc::clone(&write_arc);
    let manager_url = url.clone();

    let inner = Arc::new(UrlSessionInner {
        url: url.clone(),
        conn: Mutex::new(conn),
        write: Arc::clone(&write_arc),
        stop: stop_tx,
        in_flight: AtomicU32::new(0),
        disconnect_task: Mutex::new(None),
    });

    manager.insert(&url, Arc::clone(&write_arc)).await;

    let inner_pump = Arc::clone(&inner);
    tokio::spawn(async move {
        run_pump(
            conn_for_pump,
            pool_for_pump,
            write_for_pump,
            read,
            stop_rx,
            inner_pump,
        )
        .await;
        manager.remove(&manager_url).await;
        super::URL_SESSIONS.lock().await.remove(&manager_url);
    });

    Ok(inner)
}

async fn run_pump(
    conn: Connection,
    pool: sqlx::SqlitePool,
    write: SharedSplitSink,
    read: crate::ws_inbound::WsRead,
    mut stop_rx: watch::Receiver<bool>,
    inner: Arc<UrlSessionInner>,
) {
    let url = conn.url.clone();
    let on_disconnect = Arc::new(move || {
        let url = url.clone();
        tokio::spawn(async move {
            super::URL_SESSIONS.lock().await.remove(&url);
        });
    });

    let mut pump = InboundPump::start(read, Arc::clone(&write), on_disconnect);
    let on_settled: Arc<dyn Fn(&Connection) + Send + Sync> = Arc::new({
        let pool = pool.clone();
        let inner = Arc::clone(&inner);
        move |c: &Connection| {
            let pool = pool.clone();
            let inner = Arc::clone(&inner);
            let c = c.clone();
            tokio::spawn(async move {
                super::work_finished(&c, &pool, &inner).await;
            });
        }
    });

    loop {
        if *stop_rx.borrow() {
            break;
        }

        let text = tokio::select! {
            _ = stop_rx.changed() => {
                if *stop_rx.borrow() {
                    break;
                }
                continue;
            }
            msg = pump.recv() => msg,
        };

        let Some(text) = text else {
            break;
        };

        let active = inner.conn.lock().await.clone();
        if let Err(e) = protocol::handle_server_message(
            &active,
            &text,
            Some(&write),
            Some(&pool),
            Some(Arc::clone(&on_settled)),
        )
        .await
        {
            warn!("[{}] Message handler error: {}", active.slug, e);
        }
    }

    pump.shutdown(&write).await;
    let intentional = *stop_rx.borrow();
    debug!(
        "{}",
        i18n::t_fmt("session.pump_stopped", &[("slug", &conn.slug)]),
    );

    super::link::notify_pump_stopped(intentional, inner.url.clone(), pool);
}

pub async fn disconnect(inner: &UrlSessionInner) {
    let _ = inner.stop.send(true);
    crate::ws_inbound::close_write_half(&inner.write).await;
    inner.disconnect_task.lock().await.take().map(|h| h.abort());
    super::URL_SESSIONS.lock().await.remove(&inner.url);
    debug!(
        "{}",
        i18n::t_fmt(
            "session.disconnected",
            &[("slug", &inner.conn.lock().await.slug)],
        )
    );
}

pub async fn schedule_disconnect_after_idle(
    conn: Connection,
    pool: sqlx::SqlitePool,
    inner: Arc<UrlSessionInner>,
) {
    let mut guard = inner.disconnect_task.lock().await;
    if let Some(handle) = guard.take() {
        handle.abort();
    }

    let inner_clone = Arc::clone(&inner);
    *guard = Some(tokio::spawn(async move {
        tokio::time::sleep(DISCONNECT_GRACE).await;

        if inner_clone.in_flight.load(Ordering::SeqCst) > 0
            || pending::has_any(&conn.url).await
            || recovery::is_active(&conn.url)
        {
            return;
        }

        match crate::cron::next_task_within_for_url(&pool, &conn.url, SESSION_HOLD).await {
            Ok(true) => {
                debug!(
                    "{}",
                    i18n::t_fmt(
                        "session.holding",
                        &[("slug", &conn.slug), ("min", SESSION_HOLD_MINUTES)],
                    )
                );
            }
            Ok(false) => {
                disconnect(&inner_clone).await;
            }
            Err(e) => {
                warn!("[{}] Session idle check failed: {}", conn.slug, e);
            }
        }
    }));
}

