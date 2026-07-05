//! config.toml load, env overrides, first-run prompts.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::Path;

use crate::i18n;
use crate::paths;
use crate::tls;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    #[serde(default)]
    pub device_id: String,

    #[serde(default = "default_server_addr")]
    pub server_addr: String,

    #[serde(default = "default_public_ip")]
    pub public_ip: String,

    #[serde(default = "default_server_port")]
    pub server_port: u16,

    #[serde(default = "default_files_dir")]
    pub files_dir: String,

    #[serde(default = "default_scripts_dir")]
    pub scripts_dir: String,

    #[serde(default = "default_debug")]
    pub debug: bool,

    #[serde(default = "default_mysqldump_path")]
    pub mysqldump_path: String,

    #[serde(default = "default_pg_dump_path")]
    pub pg_dump_path: String,

    #[serde(default = "default_tls_cert")]
    pub tls_cert: String,

    #[serde(default = "default_tls_key")]
    pub tls_key: String,

    #[serde(default = "default_encrypt_backups")]
    pub encrypt_backups: bool,

    #[serde(default = "default_encrypt_password")]
    pub encrypt_password: String,
}

fn default_tls_cert() -> String {
    "tls/server.crt".to_string()
}
fn default_tls_key() -> String {
    "tls/server.key".to_string()
}

fn default_public_ip() -> String {
    "127.0.0.1".to_string()
}
fn default_server_addr() -> String {
    "0.0.0.0".to_string()
}
fn default_server_port() -> u16 {
    8080
}
fn default_files_dir() -> String {
    "data/temp".to_string()
}
fn default_scripts_dir() -> String {
    "data/scripts".to_string()
}
fn default_debug() -> bool {
    false
}
fn default_mysqldump_path() -> String {
    "mysqldump".to_string()
}
fn default_pg_dump_path() -> String {
    "pg_dump".to_string()
}

fn default_encrypt_backups() -> bool {
    false
}
fn default_encrypt_password() -> String {
    String::new()
}

fn default_device_id() -> String {
    String::new()
}

impl Config {
    pub fn load_or_init() -> Result<Self> {
        let config_path = paths::config_path()?;

        if !config_path.exists() && !tls::files_present_at_default_paths()? {
            return Err(setup_no_bundle_error(&config_path));
        }

        let (mut config, is_new) = if config_path.exists() {
            let content = fs::read_to_string(&config_path)
                .with_context(|| format!("Failed to read config: {}", paths::display_path(&config_path)))?;
            let content = prepare_toml_content(&content);
            let mut config: Config = parse_config_toml(&content)?;
            normalize_tool_paths(&mut config);
            migrate_legacy_logger(&mut config, &content);
            (config, false)
        } else {
            let config = Config {
                device_id: default_device_id(),
                server_addr: default_server_addr(),
                public_ip: default_public_ip(),
                server_port: default_server_port(),
                files_dir: default_files_dir(),
                scripts_dir: default_scripts_dir(),
                debug: default_debug(),
                mysqldump_path: default_mysqldump_path(),
                pg_dump_path: default_pg_dump_path(),
                tls_cert: default_tls_cert(),
                tls_key: default_tls_key(),
                encrypt_backups: default_encrypt_backups(),
                encrypt_password: default_encrypt_password(),
            };

            eprintln!(
                "{}",
                i18n::t_fmt(
                    "setup.new_config",
                    &[("path", &paths::display_path(&config_path))]
                )
            );
            eprintln!("{}", i18n::t("setup.device_id_hint"));
            (config, true)
        };

        config.ensure_directories()?;

        if is_new && !tls::files_present(&config)? {
            return Err(anyhow::anyhow!("{}", tls::missing_tls_message(&config)));
        }

        let mut needs_save = is_new;
        if is_local_public_host(&config.public_ip) {
            if let Some((host, port)) = prompt_public_ip()? {
                config.public_ip = host;
                if let Some(p) = port {
                    config.server_port = p;
                }
                needs_save = true;
            }
        }

        if needs_save {
            config.save(&config_path)?;
            if is_new {
                eprintln!(
                    "{}",
                    i18n::t_fmt(
                        "setup.config_saved",
                        &[("path", &paths::display_path(&config_path))]
                    )
                );
            }
        }

        Ok(config)
    }

    pub fn download_scheme(&self) -> &'static str {
        "https"
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let body = toml::to_string_pretty(self).context("Failed to serialize config")?;
        let content = format!(
            "# Backup server configuration\n\
             # device_id — empty until first client connects; clear to allow a new client PC.\n\
             # debug — false: normal logs (info), true: verbose (debug) + memory monitor.\n\
             # encrypt_password — AES key for tasks with Encrypt mode on (set per task on client).\n\
             # encrypt_backups — legacy fallback when a synced task has no encrypt field.\n\
             # public_ip — external IP or hostname for client connections (WebSocket, downloads).\n\
             #             You can change any value here; restart the server to apply.\n\n\
             {body}"
        );
        fs::write(path, content)
            .with_context(|| format!("Failed to write config: {}", paths::display_path(path)))?;
        Ok(())
    }

    pub fn is_debug(&self) -> bool {
        self.debug
    }

    pub fn has_encrypt_key(&self) -> bool {
        !self.encrypt_password.is_empty()
    }

    pub fn server_addr(&self) -> String {
        format!("{}:{}", self.public_ip, self.server_port)
    }

    pub fn files_dir_abs(&self) -> Result<std::path::PathBuf> {
        paths::resolve(&self.files_dir)
    }

    pub fn scripts_dir_abs(&self) -> Result<std::path::PathBuf> {
        paths::resolve(&self.scripts_dir)
    }

    fn ensure_directories(&self) -> Result<()> {
        for dir in [&self.files_dir, &self.scripts_dir] {
            let full = paths::resolve(dir)?;
            fs::create_dir_all(&full)
                .with_context(|| format!("Failed to create directory: {}", paths::display_path(&full)))?;
        }
        if let Ok(tls_dir) = paths::resolve("tls") {
            fs::create_dir_all(&tls_dir)
                .with_context(|| format!("Failed to create directory: {}", paths::display_path(&tls_dir)))?;
        }
        Ok(())
    }
}

fn parse_config_toml(raw: &str) -> Result<Config> {
    let mut value: toml::Value = toml::from_str(raw).map_err(|e| {
        if cfg!(windows) {
            anyhow::anyhow!(
                "{e}\n\
                 On Windows, use forward slashes in paths (e.g. D:/tools/mysqldump.exe) \
                 or single-quoted strings ('D:\\tools\\mysqldump.exe')."
            )
        } else {
            anyhow::anyhow!("{e}")
        }
    })?;
    if let Some(table) = value.as_table_mut() {
        table.remove("api_key");
        if !table.contains_key("device_id") {
            table.insert("device_id".to_string(), toml::Value::String(String::new()));
        }
        normalize_legacy_bools(table);
    }
    value
        .try_into()
        .context("Failed to parse config.toml fields")
}

/// On Windows, fix `C:\path` in double-quoted TOML strings (invalid `\O` escapes).
#[cfg(windows)]
fn prepare_toml_content(raw: &str) -> String {
    fix_windows_slashes_in_toml_strings(raw)
}

#[cfg(not(windows))]
fn prepare_toml_content(raw: &str) -> String {
    raw.to_string()
}

#[cfg(windows)]
fn fix_windows_slashes_in_toml_strings(raw: &str) -> String {
    // In double-quoted TOML strings, `\b`, `\t`, `\n` etc. are escape sequences.
    // Windows paths like `D:\tools\bin\mysqldump.exe` must be normalized
    // before parse — every `\` → `/` inside quotes (paths never need TOML escapes).
    let mut out = String::with_capacity(raw.len());
    let mut in_quotes = false;
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            in_quotes = !in_quotes;
            out.push(c);
            i += 1;
            continue;
        }
        if in_quotes && c == '\\' {
            if i + 1 < chars.len() && chars[i + 1] == '\\' {
                out.push('/');
                i += 2;
                continue;
            }
            out.push('/');
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

fn normalize_tool_paths(config: &mut Config) {
    config.mysqldump_path = normalize_path_field(&config.mysqldump_path);
    config.pg_dump_path = normalize_path_field(&config.pg_dump_path);
}

fn normalize_path_field(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // Strip control chars (e.g. \x08 from `\b` in a previously broken TOML parse).
    let cleaned: String = trimmed.chars().filter(|c| !c.is_control()).collect();
    if cleaned.contains('\\') || cleaned.contains('/') || cleaned.contains(':') {
        std::path::Path::new(&cleaned)
            .to_string_lossy()
            .to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn windows_toml_path_fixup_avoids_b_escape() {
        let raw = r#"mysqldump_path = "D:\mysql\bin\mysqldump.exe""#;
        let fixed = fix_windows_slashes_in_toml_strings(raw);
        assert!(fixed.contains("mysql/bin/mysqldump.exe"));
        assert!(!fixed.contains("\\b"));
        let config: Config = toml::from_str(&fixed).expect("parse fixed toml");
        assert_eq!(config.mysqldump_path, "D:/mysql/bin/mysqldump.exe");
    }
}

fn migrate_legacy_logger(config: &mut Config, raw: &str) {
    let Ok(value) = toml::from_str::<toml::Value>(raw) else {
        return;
    };
    let Some(table) = value.as_table() else {
        return;
    };
    if table.contains_key("debug") {
        return;
    }
    if let Some(logger) = table.get("logger").and_then(|v| v.as_str()) {
        config.debug = logger.eq_ignore_ascii_case("full");
    }
}

fn normalize_legacy_bools(table: &mut toml::map::Map<String, toml::Value>) {
    if let Some(debug) = table.get("debug") {
        let as_bool = match debug {
            toml::Value::Integer(0) => false,
            toml::Value::Integer(1) => true,
            toml::Value::Integer(n) => *n != 0,
            toml::Value::Boolean(b) => *b,
            toml::Value::String(s) => {
                let s = s.trim();
                s == "1" || s.eq_ignore_ascii_case("true")
            }
            _ => false,
        };
        table.insert("debug".to_string(), toml::Value::Boolean(as_bool));
    }
    if let Some(enc) = table.get("encrypt_backups") {
        let as_bool = match enc {
            toml::Value::Integer(0) => false,
            toml::Value::Integer(1) => true,
            toml::Value::Integer(n) => *n != 0,
            toml::Value::Boolean(b) => *b,
            toml::Value::String(s) => {
                let s = s.trim();
                s == "1" || s.eq_ignore_ascii_case("true")
            }
            _ => false,
        };
        table.insert("encrypt_backups".to_string(), toml::Value::Boolean(as_bool));
    }
}

fn setup_no_bundle_error(config_path: &Path) -> anyhow::Error {
    let cert = paths::resolve("tls/server.crt")
        .map(|p| paths::display_path(&p))
        .unwrap_or_else(|_| "tls/server.crt".to_string());
    let key = paths::resolve("tls/server.key")
        .map(|p| paths::display_path(&p))
        .unwrap_or_else(|_| "tls/server.key".to_string());
    let config = paths::display_path(config_path);
    let msg = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
        i18n::t("setup.no_bundle_title"),
        i18n::t("setup.no_bundle_body"),
        i18n::t("setup.no_bundle_step1"),
        i18n::t("setup.no_bundle_step2"),
        i18n::t("setup.no_bundle_step3"),
        i18n::t("setup.no_bundle_step4"),
        i18n::t("setup.no_bundle_paths"),
        i18n::t_fmt("setup.no_bundle_config", &[("config", &config)]),
        i18n::t_fmt("setup.no_bundle_cert", &[("cert", &cert)]),
        i18n::t_fmt("setup.no_bundle_key", &[("key", &key)]),
        i18n::t("tls.missing_how"),
    );
    anyhow::anyhow!(msg)
}

fn read_stdin_line() -> Result<String> {
    let mut buf = Vec::new();
    io::stdin()
        .lock()
        .read_until(b'\n', &mut buf)
        .map_err(|e| {
            anyhow::anyhow!(
                "{}",
                i18n::t_fmt("setup.stdin_error", &[("detail", &e.to_string())])
            )
        })?;
    Ok(String::from_utf8_lossy(&buf)
        .trim_end_matches(|c| c == '\n' || c == '\r')
        .to_string())
}

fn is_local_public_host(host: &str) -> bool {
    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() || host == "localhost" {
        return true;
    }
    if host == "::1" {
        return true;
    }
    if let Ok(addr) = host.parse::<std::net::IpAddr>() {
        return addr.is_loopback();
    }
    false
}

fn is_valid_public_host(host: &str) -> bool {
    let host = host.trim();
    if host.is_empty() || host.contains(char::is_whitespace) {
        return false;
    }
    if host.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    host.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

fn prompt_public_ip() -> Result<Option<(String, Option<u16>)>> {
    if !io::stdin().is_terminal() {
        eprintln!("{}", i18n::t("setup.public_ip_non_tty"));
        return Ok(None);
    }

    loop {
        i18n::print_key("setup.public_ip_prompt");
        io::stdout().flush()?;

        let value = read_stdin_line()?;

        match parse_public_endpoint(&value) {
            Ok((host, port)) => {
                match port {
                    Some(p) => println!(
                        "{}",
                        i18n::t_fmt(
                            "setup.public_ip_saved_port",
                            &[("host", &host), ("port", &p.to_string())]
                        )
                    ),
                    None => println!(
                        "{}",
                        i18n::t_fmt("setup.public_ip_saved", &[("host", &host)])
                    ),
                }
                return Ok(Some((host, port)));
            }
            Err(e) => eprintln!(
                "{}",
                i18n::t_fmt("common.arrow_hint", &[("msg", &e.to_string())])
            ),
        }
    }
}

fn parse_public_endpoint(input: &str) -> Result<(String, Option<u16>)> {
    let input = input.trim();
    if input.is_empty() {
        anyhow::bail!("{}", i18n::t("setup.public_ip_required"));
    }

    let (host, port) = if let Some((host, port_str)) = input.rsplit_once(':') {
        if host.is_empty() {
            anyhow::bail!("{}", i18n::t("setup.public_ip_invalid_host"));
        }
        let port: u16 = port_str.parse().map_err(|_| {
            anyhow::anyhow!(
                "{}",
                i18n::t_fmt("setup.public_ip_invalid_port", &[("port", port_str)])
            )
        })?;
        (host.to_string(), Some(port))
    } else {
        (input.to_string(), None)
    };

    if !is_valid_public_host(&host) {
        anyhow::bail!("{}", i18n::t("setup.public_ip_invalid"));
    }
    if is_local_public_host(&host) {
        anyhow::bail!("{}", i18n::t("setup.public_ip_no_localhost"));
    }

    Ok((host, port))
}
