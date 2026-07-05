//! Main menu + folder browser for `.aes` files.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use dialoguer::{theme::ColorfulTheme, Confirm, Input, Select};
use rand::Rng;

use crate::config::{self, DecryptConfig};
use crate::crypto;

const PASSWORD_CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";

enum EntryKind {
    Parent,
    Dir(PathBuf),
    AesFile(PathBuf),
    BackToMenu,
}

struct DirListing {
    labels: Vec<String>,
    kinds: Vec<EntryKind>,
}

fn pause() {
    let _ = Input::<String>::new()
        .with_prompt("Press Enter")
        .allow_empty(true)
        .interact();
}

fn should_persist(cfg: &DecryptConfig) -> bool {
    !cfg.profiles.is_empty()
        || !cfg.path.trim().is_empty()
        || !cfg.output_path.trim().is_empty()
        || !cfg.last_path.trim().is_empty()
}

fn persist_config(cfg_path: &Path, cfg: &DecryptConfig) {
    if !should_persist(cfg) {
        return;
    }
    if let Err(e) = config::save(cfg_path, cfg) {
        println!("\n  Could not save config: {e:#}");
        pause();
    }
}

fn print_banner(cfg: &DecryptConfig, cfg_path: &Path) {
    crate::banner::print_logo();
    println!();
    println!("  Config:   {}", cfg_path.display());
    if !cfg_path.exists() {
        println!("  (config will be created when you save a connection or setting)");
    }
    println!("  Connections: {}", cfg.profiles_hint());
    println!("  Root:     {}", cfg.path_hint());
    println!("  Output:   {}", cfg.output_path_hint());
    if !cfg.is_ready() {
        println!();
        println!("  >> List connections → Add connection, then Settings → backups root folder.");
        println!("     Layout: {{root}}/{{connection-name}}/backup_*.zip.aes");
    }
    println!();
}

fn prompt_connection_name(default: &str) -> anyhow::Result<String> {
    let name: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Connection name (same as in backup-client, e.g. production)")
        .default(default.to_string())
        .interact_text()?;
    let name = name.trim().to_string();
    if name.is_empty() {
        anyhow::bail!("Connection name cannot be empty");
    }
    if name.contains('/') || name.contains('\\') {
        anyhow::bail!("Connection name must not contain path separators");
    }
    Ok(name)
}

fn prompt_password_visible(prompt: &str) -> anyhow::Result<String> {
    let pass: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt(prompt)
        .interact_text()?;
    let pass = pass.trim().to_string();
    if pass.is_empty() {
        anyhow::bail!("Password cannot be empty");
    }
    Ok(pass)
}

fn generate_random_password(len: usize) -> String {
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| PASSWORD_CHARS[rng.gen_range(0..PASSWORD_CHARS.len())] as char)
        .collect()
}

fn print_password_setup_hint(slug: &str, password: &str) {
    println!();
    println!("  Connection: {slug}");
    println!("  Password:   {password}");
    println!();
    println!("  Copy this password to the server → config.toml → encrypt_password");
    println!("  Enable Encrypt mode on tasks; restart backup-server.");
}

fn add_connection(cfg: &mut DecryptConfig, cfg_path: &Path) -> anyhow::Result<()> {
    println!();
    println!("  A random password is generated for each new connection.");
    println!("  You copy the same value to encrypt_password on that VPS.");
    let name = prompt_connection_name("")?;
    if cfg.profile_password(&name).is_some() {
        anyhow::bail!("Connection '{name}' already exists — open it from the list to edit");
    }
    let pass = generate_random_password(28);
    print_password_setup_hint(&name, &pass);
    cfg.set_profile_password(&name, pass);
    persist_config(cfg_path, cfg);
    println!();
    println!("  Saved to {}.", cfg_path.display());
    pause();
    Ok(())
}

fn change_connection_password(
    cfg: &mut DecryptConfig,
    cfg_path: &Path,
    slug: &str,
) -> anyhow::Result<()> {
    println!();
    println!("  Must match encrypt_password on that backup-server.");
    let pass = prompt_password_visible("New password")?;
    cfg.set_profile_password(slug, pass);
    persist_config(cfg_path, cfg);
    println!("  Password updated for '{slug}'.");
    pause();
    Ok(())
}

fn regenerate_connection_password(
    cfg: &mut DecryptConfig,
    cfg_path: &Path,
    slug: &str,
) -> anyhow::Result<()> {
    let pass = generate_random_password(28);
    print_password_setup_hint(slug, &pass);
    if Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(format!("Save new password for '{slug}'?"))
        .default(true)
        .interact()?
    {
        cfg.set_profile_password(slug, pass);
        persist_config(cfg_path, cfg);
        println!("  Saved.");
    }
    pause();
    Ok(())
}

fn rename_connection(
    cfg: &mut DecryptConfig,
    cfg_path: &Path,
    old: &str,
) -> anyhow::Result<Option<String>> {
    println!();
    let new = prompt_connection_name(old)?;
    if new == old {
        return Ok(None);
    }
    if cfg.profile_password(&new).is_some() {
        anyhow::bail!("Connection '{new}' already exists");
    }
    let pass = cfg
        .profiles
        .remove(old)
        .ok_or_else(|| anyhow::anyhow!("Connection not found"))?;
    cfg.set_profile_password(&new, pass);
    persist_config(cfg_path, cfg);
    println!("  Renamed '{old}' → '{new}'.");
    pause();
    Ok(Some(new))
}

fn delete_connection(cfg: &mut DecryptConfig, cfg_path: &Path, slug: &str) -> anyhow::Result<()> {
    if Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(format!("Delete connection '{slug}'?"))
        .default(false)
        .interact()?
    {
        cfg.remove_profile(slug);
        persist_config(cfg_path, cfg);
        println!("  Deleted.");
    }
    pause();
    Ok(())
}

fn connection_detail_menu(
    cfg: &mut DecryptConfig,
    cfg_path: &Path,
    slug: String,
) -> anyhow::Result<()> {
    let mut slug = slug;
    loop {
        let items = [
            "Change name",
            "Change password",
            "Regenerate password",
            "Delete connection",
            "← Back",
        ];
        let sel = Select::with_theme(&ColorfulTheme::default())
            .with_prompt(format!("Connection: {slug}"))
            .items(&items)
            .default(0)
            .interact()?;

        match sel {
            0 => match rename_connection(cfg, cfg_path, &slug) {
                Ok(Some(new)) => slug = new,
                Ok(None) => {}
                Err(e) => {
                    println!("\n  {e:#}");
                    pause();
                }
            },
            1 => {
                if let Err(e) = change_connection_password(cfg, cfg_path, &slug) {
                    println!("\n  {e:#}");
                }
            }
            2 => {
                if let Err(e) = regenerate_connection_password(cfg, cfg_path, &slug) {
                    println!("\n  {e:#}");
                }
            }
            3 => {
                delete_connection(cfg, cfg_path, &slug)?;
                return Ok(());
            }
            _ => return Ok(()),
        }
    }
}

fn list_connections_menu(cfg: &mut DecryptConfig, cfg_path: &Path) -> anyhow::Result<()> {
    loop {
        let slugs = cfg.profile_slugs();
        let mut labels = vec!["Add connection".to_string()];
        labels.extend(slugs.iter().map(|s| format!("{s} (password set)")));
        labels.push("← Back".to_string());

        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let sel = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("List connections")
            .items(&refs)
            .default(0)
            .interact()?;

        if sel == 0 {
            if let Err(e) = add_connection(cfg, cfg_path) {
                println!("\n  {e:#}");
                pause();
            }
        } else if sel == labels.len() - 1 {
            return Ok(());
        } else {
            let slug = slugs[sel - 1].clone();
            connection_detail_menu(cfg, cfg_path, slug)?;
        }
    }
}

fn settings_menu(cfg: &mut DecryptConfig, cfg_path: &Path) -> anyhow::Result<()> {
    loop {
        let items = [
            "Set backups root folder",
            "Set output path for decrypted files",
            "← Back",
        ];
        let sel = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Settings")
            .items(&items)
            .default(0)
            .interact()?;

        match sel {
            0 => {
                if let Err(e) = set_root_path(cfg) {
                    println!("\n  {e:#}");
                } else {
                    persist_config(cfg_path, cfg);
                }
            }
            1 => {
                if let Err(e) = set_output_path(cfg) {
                    println!("\n  {e:#}");
                } else {
                    persist_config(cfg_path, cfg);
                }
            }
            _ => return Ok(()),
        }
    }
}

fn set_root_path(cfg: &mut DecryptConfig) -> anyhow::Result<()> {
    println!();
    println!("  Root folder with connection subfolders and *.aes files.");
    println!("  Example: /backups/production/backup_foo.zip.aes");
    let default = cfg.path.clone();
    let path: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Backups root folder")
        .default(default)
        .interact_text()?;
    let path = path.trim().to_string();
    if path.is_empty() {
        anyhow::bail!("Path cannot be empty");
    }
    let p = PathBuf::from(&path);
    if !p.exists() {
        let create = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt(format!("Folder does not exist. Create {}?", p.display()))
            .default(false)
            .interact()?;
        if create {
            fs::create_dir_all(&p).with_context(|| format!("Create {}", p.display()))?;
        } else {
            anyhow::bail!("Folder not found");
        }
    }
    cfg.path = path;
    println!("  Root folder saved.");
    pause();
    Ok(())
}

fn set_output_path(cfg: &mut DecryptConfig) -> anyhow::Result<()> {
    println!();
    println!("  Where to save decrypted files (plaintext).");
    println!("  Empty = next to the .aes file (default).");
    let default = cfg.output_path.clone();
    let path: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Output path for decrypted files")
        .default(default)
        .allow_empty(true)
        .interact_text()?;
    let path = path.trim().to_string();
    if path.is_empty() {
        cfg.output_path.clear();
        println!("  Output path cleared (decrypt in-place).");
        pause();
        return Ok(());
    }
    let p = PathBuf::from(&path);
    if !p.exists() {
        let create = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt(format!("Folder does not exist. Create {}?", p.display()))
            .default(true)
            .interact()?;
        if create {
            fs::create_dir_all(&p).with_context(|| format!("Create {}", p.display()))?;
        } else {
            anyhow::bail!("Folder not found");
        }
    }
    p.canonicalize().with_context(|| format!("output_path '{path}'"))?;
    cfg.output_path = path;
    println!("  Output path saved.");
    pause();
    Ok(())
}

fn list_directory(current: &Path, root: &Path) -> anyhow::Result<DirListing> {
    let mut labels = Vec::new();
    let mut kinds = Vec::new();

    let can_current = current.canonicalize().unwrap_or_else(|_| current.to_path_buf());
    let can_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());

    if can_current != can_root {
        labels.push("..  (back)".to_string());
        kinds.push(EntryKind::Parent);
    }

    let mut dirs = Vec::new();
    let mut files = Vec::new();

    for entry in fs::read_dir(current).with_context(|| format!("Read dir {}", current.display()))? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            dirs.push((name, path));
        } else if name.ends_with(".aes") {
            files.push((name, path));
        }
    }

    dirs.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    files.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));

    for (name, path) in dirs {
        labels.push(format!("[DIR]  {name}/"));
        kinds.push(EntryKind::Dir(path));
    }

    for (name, path) in files {
        labels.push(format!("[AES]  {name}"));
        kinds.push(EntryKind::AesFile(path));
    }

    labels.push("← Back to main menu".to_string());
    kinds.push(EntryKind::BackToMenu);

    Ok(DirListing { labels, kinds })
}

fn pick_profile_password(
    cfg: &DecryptConfig,
    enc_path: &Path,
    root: &Path,
    exclude: &[String],
) -> anyhow::Result<Option<String>> {
    let mut items: Vec<String> = Vec::new();
    let mut slugs: Vec<String> = Vec::new();

    if let Some(slug) = config::slug_from_path(enc_path, root) {
        if cfg.profile_password(&slug).is_some() && !exclude.iter().any(|e| e == &slug) {
            items.push(format!("Auto: {slug} (from folder path)"));
            slugs.push(slug);
        }
    }

    for slug in cfg.profile_slugs() {
        if exclude.contains(&slug) || slugs.contains(&slug) {
            continue;
        }
        items.push(format!("{slug} (password set)"));
        slugs.push(slug);
    }

    items.push("Enter password once (not saved)".to_string());
    items.push("Cancel".to_string());

    if items.len() <= 2 {
        anyhow::bail!("No connections configured — use List connections → Add connection");
    }

    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    let sel = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select connection / password")
        .items(&refs)
        .default(0)
        .interact()?;

    if sel == items.len() - 1 {
        return Ok(None);
    }
    if sel == items.len() - 2 {
        return prompt_password_visible("Decryption password").map(Some);
    }

    Ok(cfg.password_for_slug(&slugs[sel]))
}

fn try_decrypt_with_password(
    enc_path: &Path,
    cfg: &DecryptConfig,
    password: &str,
) -> anyhow::Result<String> {
    let name = enc_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow::anyhow!("Bad filename"))?;
    let plain_name = crypto::plaintext_filename(name)
        .ok_or_else(|| anyhow::anyhow!("Expected .aes extension: {name}"))?;
    let out_path = config::resolve_plaintext_path(enc_path, &plain_name, cfg)?;

    if out_path.exists() {
        let ok = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt(format!("Overwrite {}?", out_path.display()))
            .default(false)
            .interact()?;
        if !ok {
            anyhow::bail!("Skipped");
        }
    }

    let size = crypto::decrypt_file(enc_path, &out_path, password)?;
    Ok(format!("{} ({} bytes)", out_path.display(), size))
}

fn decrypt_aes_file(enc_path: &Path, cfg: &DecryptConfig, root: &Path) -> anyhow::Result<String> {
    let mut tried_slugs: Vec<String> = Vec::new();
    let mut use_auto = true;

    loop {
        let password = if use_auto {
            if let Some(p) = cfg.auto_password_for_file(enc_path, root) {
                let slug = config::slug_from_path(enc_path, root);
                if slug.as_ref().is_some_and(|s| tried_slugs.contains(s)) {
                    pick_profile_password(cfg, enc_path, root, &tried_slugs)?
                        .ok_or_else(|| anyhow::anyhow!("Cancelled"))?
                } else {
                    if let Some(ref s) = slug {
                        println!("  Using connection: {s}");
                    } else if cfg.profiles.len() == 1 {
                        println!("  Using connection: {}", cfg.profile_slugs()[0]);
                    }
                    p
                }
            } else {
                pick_profile_password(cfg, enc_path, root, &tried_slugs)?
                    .ok_or_else(|| anyhow::anyhow!("Cancelled"))?
            }
        } else {
            pick_profile_password(cfg, enc_path, root, &tried_slugs)?
                .ok_or_else(|| anyhow::anyhow!("Cancelled"))?
        };

        match try_decrypt_with_password(enc_path, cfg, &password) {
            Ok(msg) => return Ok(msg),
            Err(e) if e.to_string() == "Skipped" => return Err(e),
            Err(e) if crypto::is_wrong_password(&e) => {
                println!("\n  Wrong password for this file.");
                use_auto = false;
                if let Some(slug) = config::slug_from_path(enc_path, root) {
                    if !tried_slugs.contains(&slug) {
                        tried_slugs.push(slug);
                    }
                } else if cfg.profiles.len() == 1 {
                    let slug = cfg.profile_slugs()[0].clone();
                    if !tried_slugs.contains(&slug) {
                        tried_slugs.push(slug);
                    }
                }
                let other_count = cfg.profiles.len().saturating_sub(tried_slugs.len());
                if other_count == 0 {
                    anyhow::bail!("Decryption failed (wrong password or corrupted file)");
                }
                let retry = Confirm::with_theme(&ColorfulTheme::default())
                    .with_prompt("Try another connection?")
                    .default(true)
                    .interact()?;
                if !retry {
                    anyhow::bail!("Decryption failed (wrong password or corrupted file)");
                }
            }
            Err(e) => return Err(e),
        }
    }
}

fn run_browser(
    cfg: &mut DecryptConfig,
    cfg_path: &Path,
    root: &Path,
    mut current: PathBuf,
) -> anyhow::Result<()> {
    loop {
        println!();
        println!("── Browse .aes files ──────────────────────────────────");
        println!("  {}", current.display());
        println!("  Root: {}", root.display());
        println!("──────────────────────────────────────────────────────");

        let listing = list_directory(&current, root)?;
        if listing.labels.len() <= 1 {
            println!("  (no .aes files or subfolders here)");
        }

        let item_refs: Vec<&str> = listing.labels.iter().map(String::as_str).collect();
        let prompt = format!(
            "{} — select",
            current.file_name().unwrap_or_default().to_string_lossy()
        );
        let sel = Select::with_theme(&ColorfulTheme::default())
            .with_prompt(prompt)
            .items(&item_refs)
            .default(0)
            .interact()?;

        match &listing.kinds[sel] {
            EntryKind::BackToMenu => return Ok(()),
            EntryKind::Parent => {
                if let Some(parent) = current.parent() {
                    current = parent.to_path_buf();
                    cfg.last_path = current.display().to_string();
                    persist_config(cfg_path, cfg);
                }
            }
            EntryKind::Dir(path) => {
                current = path.clone();
                cfg.last_path = current.display().to_string();
                persist_config(cfg_path, cfg);
            }
            EntryKind::AesFile(path) => {
                match decrypt_aes_file(path, cfg, root) {
                    Ok(msg) => println!("\n  OK: decrypted → {msg}"),
                    Err(e) if e.to_string() == "Skipped" => println!("\n  Skipped."),
                    Err(e) if e.to_string() == "Cancelled" => println!("\n  Cancelled."),
                    Err(e) => println!("\n  Error: {e:#}"),
                }
                pause();
            }
        }
    }
}

/// Main loop: settings + browse in one app.
pub fn run_app(
    cfg: &mut DecryptConfig,
    cfg_path: &Path,
    cli_start_dir: Option<PathBuf>,
) -> anyhow::Result<()> {
    loop {
        print_banner(cfg, cfg_path);

        let browse_label = if cfg.is_ready() {
            "Browse .aes files"
        } else {
            "Browse .aes files  (add connection & root folder first)"
        };

        let items = [
            browse_label,
            "List connections",
            "Settings",
            "Exit",
        ];

        let sel = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Main menu")
            .items(&items)
            .default(0)
            .interact()?;

        match sel {
            0 => {
                if !cfg.is_ready() {
                    println!();
                    println!("  Add a connection and set backups root folder in Settings first.");
                    pause();
                    continue;
                }
                let root = match config::canonical_root(cfg) {
                    Ok(r) => r,
                    Err(e) => {
                        println!("\n  {e:#}");
                        pause();
                        continue;
                    }
                };
                let start = if let Some(dir) = &cli_start_dir {
                    let dir = dir
                        .canonicalize()
                        .with_context(|| format!("{}", dir.display()))?;
                    if !config::is_under_root(&dir, &root) {
                        println!("\n  Folder outside configured root.");
                        pause();
                        continue;
                    }
                    dir
                } else {
                    config::start_dir(cfg, &root)
                };
                cfg.last_path = start.display().to_string();
                persist_config(cfg_path, cfg);
                run_browser(cfg, cfg_path, &root, start)?;
            }
            1 => list_connections_menu(cfg, cfg_path)?,
            2 => settings_menu(cfg, cfg_path)?,
            _ => {
                println!();
                println!("  Goodbye.");
                break;
            }
        }
    }

    persist_config(cfg_path, cfg);
    Ok(())
}

pub fn pause_on_exit() {
    pause();
}
