//! Decrypt backup-server `.zip.aes` / `.txt.aes` files.

mod banner;
mod config;
mod crypto;
mod tui;

use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;

#[derive(Parser)]
#[command(name = "backup-decrypt", version)]
#[command(about = "Decrypt .aes backups (AES-256-GCM, BACKUPENC1 format)")]
struct Cli {
    /// Start browser in this subfolder (under configured root)
    #[arg(value_name = "DIR")]
    directory: Option<PathBuf>,

    /// Path to decrypt.toml
    #[arg(long, value_name = "FILE")]
    key_file: Option<PathBuf>,

    /// Decrypt one file and exit (no menu)
    #[arg(long, value_name = "FILE")]
    file: Option<PathBuf>,

    /// Profile slug (connection name); auto-detected from path if omitted
    #[arg(long, value_name = "SLUG")]
    profile: Option<String>,
}

fn main() {
    let exit_code = match run() {
        Ok(()) => 0,
        Err(e) => {
            eprintln!();
            eprintln!("Error: {e:#}");
            tui::pause_on_exit();
            1
        }
    };
    std::process::exit(exit_code);
}

fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let cfg_path = config::resolve_config_path(cli.key_file.as_deref());
    let config_exists = cfg_path.exists();

    let mut cfg = if config_exists {
        config::load(&cfg_path)?
    } else {
        config::DecryptConfig::empty()
    };

    if let Some(file) = cli.file {
        if !cfg.has_any_password() {
            anyhow::bail!(
                "No connection configured. Run backup-decrypt → List connections → Add connection."
            );
        }

        let root = config::canonical_root(&cfg).ok();
        let password = if let Some(slug) = cli.profile.as_deref() {
            cfg.password_for_slug(slug).with_context(|| {
                format!("No password for profile '{slug}' in decrypt.toml")
            })?
        } else if let Some(root) = root.as_deref() {
            cfg.auto_password_for_file(&file, root).with_context(|| {
                "Could not pick password — use --profile SLUG or set [profiles] in decrypt.toml"
            })?
        } else if cfg.profiles.len() == 1 {
            cfg.profiles.values().next().cloned().unwrap()
        } else {
            anyhow::bail!("Set path in decrypt.toml or use --profile SLUG");
        };

        let name = file
            .file_name()
            .and_then(|n| n.to_str())
            .context("Bad file path")?;
        let plain_name = crypto::plaintext_filename(name)
            .ok_or_else(|| anyhow::anyhow!("Expected .aes file"))?;
        let out = config::resolve_plaintext_path(&file, &plain_name, &cfg)?;
        let size = crypto::decrypt_file(&file, &out, &password)?;
        println!("Decrypted → {} ({} bytes)", out.display(), size);
        tui::pause_on_exit();
        return Ok(());
    }

    tui::run_app(&mut cfg, &cfg_path, cli.directory)
}
