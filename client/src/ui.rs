//! Terminal UI helpers (dialoguer theme, host:port parsing).

use console::{style, Style};
use dialoguer::theme::ColorfulTheme;

use crate::i18n;

pub const DONE: &str = "+";
pub const ARROW: &str = ">";

pub fn section(title: &str) -> String {
    format!("? {title} {ARROW}")
}

pub fn done_line(label: &str, value: &str) -> String {
    format!("{DONE} {label} · {value}")
}

pub fn menu_theme() -> ColorfulTheme {
    let mut theme = ColorfulTheme::default();
    theme.active_item_prefix = style(">".to_string()).for_stderr().green().bold();
    theme.active_item_style = Style::new().for_stderr().green().bold();
    theme
}

pub fn select(prompt: &str, items: &[&str]) -> dialoguer::Result<usize> {
    dialoguer::Select::with_theme(&menu_theme())
        .with_prompt(prompt)
        .items(items)
        .interact()
}

pub fn select_default(prompt: &str, items: &[&str], default: usize) -> dialoguer::Result<usize> {
    dialoguer::Select::with_theme(&menu_theme())
        .with_prompt(prompt)
        .items(items)
        .default(default)
        .interact()
}

pub fn press_enter() {
    println!();
    let _ = dialoguer::Input::<String>::new()
        .with_prompt(i18n::t("ui.press_enter"))
        .allow_empty(true)
        .interact_text();
}

pub fn success_block(title: &str, lines: &[&str]) {
    println!();
    println!("{} {}", DONE, title);
    for line in lines {
        println!("  {line}");
    }
}

/// After Add connection / renew: how to copy bundle files to the VPS.
pub fn print_vps_deploy_instructions(bundle_path: &std::path::Path) {
    println!();
    println!("{}", i18n::t("deploy.title"));
    println!(
        "{}",
        i18n::t_fmt(
            "deploy.bundle_path",
            &[("path", &bundle_path.display().to_string())],
        )
    );
    println!("{}", i18n::t("deploy.copy_hint"));
    println!("{}", i18n::t("deploy.line_binary"));
    println!("{}", i18n::t("deploy.line_config"));
    println!("{}", i18n::t("deploy.line_service"));
    println!("{}", i18n::t("deploy.line_tls_dir"));
    println!("{}", i18n::t("deploy.line_tls_key"));
    println!("{}", i18n::t("deploy.install_cmd"));
}

pub fn server_addr_from_url(url: &str) -> String {
    let s = url.trim();
    if s.starts_with("ws://") || s.starts_with("wss://") {
        s.trim_start_matches("ws://")
            .trim_start_matches("wss://")
            .trim_end_matches("/ws")
            .trim_end_matches('/')
            .to_string()
    } else {
        s.to_string()
    }
}

pub fn host_from_url(url: &str) -> String {
    server_addr_from_url(url)
}

pub fn parse_server_addr(input: &str) -> anyhow::Result<String> {
    let input = input.trim();
    if input.is_empty() {
        anyhow::bail!("{}", i18n::t("error.address_required"));
    }

    let normalized = server_addr_from_url(input);

    let (host, port) = if let Some((host, port_str)) = normalized.rsplit_once(':') {
        if host.is_empty() {
            anyhow::bail!("{}", i18n::t("error.invalid_host"));
        }
        let port: u16 = port_str
            .parse()
            .map_err(|_| anyhow::anyhow!("{}", i18n::t_fmt("error.invalid_port", &[("port", port_str)])))?;
        (host.to_string(), port)
    } else {
        (normalized, 8080)
    };

    if host.is_empty() {
        anyhow::bail!("{}", i18n::t("error.invalid_host"));
    }

    Ok(format!("{host}:{port}"))
}

pub fn wss_url_from_server_addr(addr: &str) -> anyhow::Result<String> {
    let addr = parse_server_addr(addr)?;
    Ok(format!("wss://{addr}/ws"))
}

pub fn prompt_server_addr(default: Option<&str>) -> anyhow::Result<String> {
    let default_addr = default.unwrap_or("127.0.0.1:8080");

    loop {
        let raw = dialoguer::Input::<String>::new()
            .with_prompt(i18n::t("ui.server_prompt"))
            .default(default_addr.to_string())
            .interact_text()?;

        match parse_server_addr(&raw) {
            Ok(addr) => return Ok(addr),
            Err(e) => println!("{}", i18n::t_fmt("common.arrow_hint", &[("msg", &e.to_string())])),
        }
    }
}
