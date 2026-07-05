//! backup-server entry: TLS HTTP, WebSocket, download, worker.

mod backup;
mod config;
mod db;
mod executor;
mod health;
mod http_download;
mod i18n;
mod logging;
mod paths;
mod run_queue;
mod shutdown;
mod state;
mod task_registry;
mod task_types;
mod temp_files;
mod tls;
#[cfg(unix)]
mod memory_monitor;
mod types;
mod utils;
mod worker;
mod ws;

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::{routing::get, Json, Router};
use axum_server::tls_rustls::RustlsConfig;
use axum_server::Handle;
use tracing::info;

use config::Config;
use i18n::t_fmt;
use logging::{init as init_logging, print_startup};
use run_queue::RunQueue;
use state::AppState;
use task_registry::TaskRegistry;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("{e:#}");
        pause_before_exit();
        std::process::exit(1);
    }
}

async fn run() -> anyhow::Result<()> {
    i18n::init();
    eprintln!(
        "{}",
        t_fmt(
            "startup.version",
            &[("version", env!("CARGO_PKG_VERSION"))]
        )
    );

    let config = Config::load_or_init()?;
    init_logging(&config)?;

    let tls_info = match tls::validate_tls_startup(&config) {
        Ok(info) => info,
        Err(e) => {
            eprintln!("{e:#}");
            pause_before_exit();
            std::process::exit(1);
        }
    };

    print_startup(&config, &tls_info);
    health::run_health_checks(&config);

    let config_path = paths::config_path()?;
    let device_id = Arc::new(tokio::sync::Mutex::new(config.device_id.clone()));
    let db_pool = db::init_db().await?;

    let state = AppState {
        config: Arc::new(config),
        config_path: Arc::new(config_path),
        device_id,
        db: db_pool,
        client_channels: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        is_connected: Arc::new(tokio::sync::Mutex::new(false)),
        ws_stop: Arc::new(tokio::sync::Mutex::new(None)),
        busy_connections: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
        task_registry: Arc::new(TaskRegistry::new()),
        run_queue: Arc::new(RunQueue::new()),
    };

    #[cfg(unix)]
    memory_monitor::spawn(state.config.clone());

    let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);
    let worker_state = state.clone();
    let mut worker_shutdown = shutdown_tx.subscribe();
    tokio::spawn(async move {
        worker::run(worker_state, &mut worker_shutdown).await;
    });

    let addr: SocketAddr = format!("{}:{}", state.config.server_addr, state.config.server_port)
        .parse()
        .expect("Invalid server address");

    let app = Router::new()
        .route("/", get(|| async { "Backup Server is running" }))
        .route("/health", get(health_handler))
        .route("/ws", get(ws::ws_handler))
        .route("/download/*filename", get(http_download::download_handler))
        .with_state(state.clone());

    let handle = Handle::new();
    let graceful = handle.clone();
    tokio::spawn(async move {
        shutdown::wait_for_shutdown_signal().await;
        info!("Shutdown signal received");
        graceful.graceful_shutdown(Some(Duration::from_secs(5)));
    });

    let rustls_config = RustlsConfig::from_config(tls::load_rustls_config(&state.config)?);
    info!("Listening on https://{} (TLS)", addr);
    axum_server::bind_rustls(addr, rustls_config)
        .handle(handle)
        .serve(app.into_make_service())
        .await?;

    let _ = shutdown_tx.send(());
    Ok(())
}

/// Keep the console open on Windows when the user double-clicks the .exe or an error occurs.
fn pause_before_exit() {
    #[cfg(windows)]
    {
        use std::io::{self, IsTerminal, Write};
        if io::stderr().is_terminal() {
            let _ = writeln!(io::stderr(), "\nPress Enter to exit...");
            let _ = io::stdin().read_line(&mut String::new());
        }
    }
}

async fn health_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "ok"}))
}
