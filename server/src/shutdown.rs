//! Graceful shutdown on Ctrl+C / SIGTERM.

use tracing::info;

#[cfg(unix)]
pub async fn wait_for_shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};

    let mut term = signal(SignalKind::terminate()).expect("Failed to install SIGTERM handler");
    let mut int = signal(SignalKind::interrupt()).expect("Failed to install SIGINT handler");

    tokio::select! {
        _ = int.recv() => info!("Received SIGINT"),
        _ = term.recv() => info!("Received SIGTERM"),
    }
}

#[cfg(not(unix))]
pub async fn wait_for_shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install Ctrl+C handler");
    info!("Received Ctrl+C");
}
