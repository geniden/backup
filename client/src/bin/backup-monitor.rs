//! backup-monitor binary: read-only TUI over backup.db (WAL).

use std::path::PathBuf;

use clap::Parser;

use backup_client::{db, monitor};

#[derive(Parser)]
#[command(name = "backup-monitor", version)]
#[command(about = "Read-only TUI dashboard for backup-client (reads backup.db via WAL)")]
struct Cli {
    #[arg(long, value_name = "PATH")]
    db: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let pool = db::open_readonly(cli.db.as_deref()).await?;
    monitor::run(pool).await
}
