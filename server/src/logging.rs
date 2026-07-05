//! Tracing setup and startup banner.

use std::fmt;

use chrono::Local;
use tracing_subscriber::fmt::{format::Writer, time::FormatTime, Layer as FmtLayer};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

use crate::config::Config;
use crate::i18n;

pub const ASCII_LOGO: &str = r#"
 ____             _                _____
|  _ \           | |              / ____|
| |_) | __ _  ___| | ___   _ _ __| (___   ___ _ ____   _____ _ __
|  _ < / _` |/ __| |/ / | | | '_ \\___ \ / _ \ '__\ \ / / _ \ '__|
| |_) | (_| | (__|   <| |_| | |_) |___) |  __/ |   \ V /  __/ |
|____/ \__,_|\___|_|\_\\__,_| .__/_____/ \___|_|    \_/ \___|_|
                            | |
                            |_|
"#;

pub const OFFICIAL_WEBSITE: &str = "https://github.com/geniden/backup";

struct ShortTime;

impl FormatTime for ShortTime {
    fn format_time(&self, w: &mut Writer<'_>) -> fmt::Result {
        write!(w, "{}", Local::now().format("%Y-%m-%d T%H:%M:%S"))
    }
}

pub fn init(config: &Config) -> anyhow::Result<()> {
    let level = if config.is_debug() { "debug" } else { "info" };
    let filter = EnvFilter::try_new(format!(
        "{level},tokio_cron_scheduler=warn"
    ))?;

    let layer = FmtLayer::new()
        .with_target(false)
        .with_file(false)
        .with_line_number(false)
        .with_timer(ShortTime)
        .with_ansi(true)
        .with_filter(filter);

    tracing_subscriber::registry().with(layer).init();
    Ok(())
}

pub fn print_startup(config: &Config, tls_info: &crate::tls::TlsInfo) {
    println!("\n{ASCII_LOGO}");
    println!("BACKUP SERVER v{}", env!("CARGO_PKG_VERSION"));
    println!("{OFFICIAL_WEBSITE}");
    println!(
        "{}",
        i18n::t_fmt("banner.server_ip", &[("addr", &config.server_addr())])
    );
    println!(
        "{}",
        i18n::t_fmt(
            "banner.tls",
            &[
                ("cn", &tls_info.subject_cn),
                ("days", &tls_info.days_remaining.to_string()),
                ("date", &tls_info.not_after.format("%Y-%m-%d").to_string()),
            ]
        )
    );
    if config.device_id.is_empty() {
        println!("{}", i18n::t("banner.device_id_pending"));
    } else if config.device_id.len() >= 64 {
        println!(
            "{}",
            i18n::t_fmt(
                "banner.device_id",
                &[
                    ("prefix", &config.device_id[..8]),
                    ("suffix", &config.device_id[56..]),
                ]
            )
        );
    } else {
        println!("DEVICE-ID: {}", config.device_id);
    }
    println!(
        "{}",
        i18n::t_fmt(
            "banner.debug",
            &[(
                "state",
                if config.is_debug() {
                    i18n::t("state.on")
                } else {
                    i18n::t("state.off")
                }
                .as_str(),
            )]
        )
    );
    println!("{}", i18n::t("banner.status"));
    println!("{}", "─".repeat(60));
}
