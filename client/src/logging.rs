//! Tracing to stdout and data/backup.log.

use std::fmt;
use std::path::Path;
use std::sync::OnceLock;

use chrono::Local;
use tracing_subscriber::fmt::{format::Writer, time::FormatTime, Layer as FmtLayer};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

static FILE_GUARD: OnceLock<tracing_appender::non_blocking::WorkerGuard> = OnceLock::new();

struct ShortTime;

impl FormatTime for ShortTime {
    fn format_time(&self, w: &mut Writer<'_>) -> fmt::Result {
        write!(w, "{}", Local::now().format("%Y-%m-%d T%H:%M:%S"))
    }
}

pub const ASCII_LOGO: &str = r#"
 ____             _                 _____ _ _            _   
|  _ \           | |               / ____| (_)          | |  
| |_) | __ _  ___| | ___   _ _ __ | |    | |_  ___ _ __ | |_ 
|  _ < / _` |/ __| |/ / | | | '_ \| |    | | |/ _ \ '_ \| __|
| |_) | (_| | (__|   <| |_| | |_) | |____| | |  __/ | | | |_ 
|____/ \__,_|\___|_|\_\\__,_| .__/ \_____|_|_|\___|_| |_|\__|
                            | |                              
                            |_|                              
"#;

/// Official project site — download binaries only from here.
pub const OFFICIAL_WEBSITE: &str = "https://github.com/geniden/backup";

pub fn init(log_path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;

    let (file_writer, guard) = tracing_appender::non_blocking(file);
    let _ = FILE_GUARD.set(guard);

    let filter = EnvFilter::try_new("info,tokio_cron_scheduler=warn,sqlx=warn")?;

    let stdout = FmtLayer::new()
        .with_target(false)
        .with_timer(ShortTime)
        .with_ansi(true)
        .with_filter(filter.clone());

    let file_layer = FmtLayer::new()
        .with_writer(file_writer)
        .with_target(false)
        .with_ansi(false)
        .with_timer(ShortTime)
        .with_filter(filter);

    tracing_subscriber::registry()
        .with(stdout)
        .with(file_layer)
        .init();

    Ok(())
}

pub fn print_banner() {
    for line in ASCII_LOGO.lines() {
        if line.trim().is_empty() {
            tracing::info!("");
        } else {
            tracing::info!("{line}");
        }
    }
    tracing::info!("BACKUP CLIENT v{}", env!("CARGO_PKG_VERSION"));
    tracing::info!("{OFFICIAL_WEBSITE}");
    tracing::info!("{}", "─".repeat(60));
}
