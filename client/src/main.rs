//! backup-client binary: interactive menu and CLI subcommands.

use clap::{Parser, Subcommand};
use futures::StreamExt;

use backup_client::{db, i18n, logging, menu, paths, protocol, scheduler, tls, ui, validation};

#[derive(Parser)]
#[command(
    name = "backup-client",
    version,
    about = "Backup Client",
    long_about = "Distributed backup scheduler and menu.\n\nTip: use ASCII characters in backup download paths for maximum compatibility. Unicode paths (e.g. /home/张三/备份/) are supported via the OS filesystem APIs on Windows and Linux."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Clone, Subcommand)]
enum Commands {
    ConnectionAdd {
        slug: String,
        server: String,
    },
    ConnectionList,
    ConnectionTest {
        slug: String,
    },
    TaskAdd {
        connection_id: String,
        task_name: String,
        task_type: String,
        data_json: String,
        schedule: String,
    },
    TaskList,
    Serve,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    paths::ensure_layout()?;

    let pool = db::init_db().await?;
    i18n::init(&pool).await?;

    match &cli.command {
        Some(Commands::ConnectionAdd { slug, server }) => {
            let (url, fingerprint, _, agent_dir) =
                backup_client::bundle::prepare_tls_connection(slug, server)?;
            let id = db::add_connection(&pool, slug, &url, Some(&fingerprint)).await?;
            println!(
                "{}",
                i18n::t_fmt(
                    "cli.added_connection",
                    &[("slug", slug), ("url", &url), ("id", &id)],
                )
            );
            println!(
                "{}",
                i18n::t_fmt(
                    "cli.server_files",
                    &[("path", &agent_dir.display().to_string())],
                )
            );
            ui::print_vps_deploy_instructions(&agent_dir);
            println!("{}", i18n::t("cli.device_id_register"));
        }
        Some(Commands::ConnectionTest { slug }) => {
            let connections = db::list_connections(&pool).await?;
            let conn = connections
                .into_iter()
                .find(|c| c.slug == *slug)
                .ok_or_else(|| {
                    anyhow::anyhow!("{}", i18n::t_fmt("error.connection_not_found", &[("slug", slug)]))
                })?;
            println!(
                "{}",
                i18n::t_fmt("cli.testing", &[("url", &conn.url)])
            );
            let (ws, _) = tls::connect_ws(&conn).await?;
            let (write, mut read) = ws.split();
            let write = std::sync::Arc::new(tokio::sync::Mutex::new(write));
            protocol::authenticate_and_sync(&conn, &pool, &mut read, &write).await?;
            backup_client::ws_inbound::graceful_close(&mut read, &write).await;
            println!("{}", i18n::t("cli.connection_ok"));
        }
        Some(Commands::ConnectionList) => {
            let connections = db::list_all_connections(&pool).await?;
            if connections.is_empty() {
                println!("{}", i18n::t("cli.no_connections"));
            } else {
                for conn in &connections {
                    let st = i18n::state_on_off(conn.enabled);
                    println!(
                        "  {} | {} | {} | id={}",
                        conn.slug,
                        ui::server_addr_from_url(&conn.url),
                        st,
                        conn.id
                    );
                }
            }
        }
        Some(Commands::TaskAdd {
            connection_id,
            task_name,
            task_type,
            data_json,
            schedule,
        }) => {
            let id = format!("task_{}", uuid::Uuid::new_v4().simple());
            let task_type = validation::normalize_task_type(task_type).to_string();
            db::add_task(
                &pool,
                &id,
                connection_id,
                task_name,
                &task_type,
                data_json,
                schedule,
                true,
            )
            .await?;
            println!(
                "{}",
                i18n::t_fmt("cli.task_added", &[("name", task_name), ("id", &id)])
            );
        }
        Some(Commands::TaskList) => {
            let tasks = db::list_tasks(&pool).await?;
            for task in &tasks {
                let last_run = task
                    .last_run
                    .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                    .unwrap_or_else(|| i18n::t("cli.last_never"));
                println!(
                    "  {} [{}] {} | {} | last: {}",
                    task.id, task.task_type, task.task_name, task.schedule, last_run
                );
            }
        }
        Some(Commands::Serve) => {
            logging::init(&paths::log_path())?;
            logging::print_banner();
            scheduler::start_scheduler().await;
        }
        None => {
            logging::init(&paths::log_path())?;
            logging::print_banner();
            menu::run(&pool).await?;
        }
    }

    Ok(())
}
