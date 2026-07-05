//! Interactive terminal menu (connections, tasks, scheduler).

use std::path::PathBuf;

use dialoguer::Confirm;
use futures::StreamExt;

use crate::bundle;
use crate::ca;
use crate::db;
use crate::i18n;
use crate::models::connection::Connection;
use crate::models::task::Task;
use crate::paths;
use crate::protocol;
use crate::runner;
use crate::task_params;
use crate::tls;
use crate::ui;
use crate::validation::prompt_slug;

pub async fn run(pool: &sqlx::SqlitePool) -> anyhow::Result<()> {
    loop {
        let connections = db::list_all_connections(pool).await?;
        let mut items: Vec<String> = vec![i18n::t("menu.add_connection")];
        for c in &connections {
            let st = i18n::state_on_off(c.enabled);
            items.push(format!("  {} [{}]", c.label(), st));
        }
        items.push(i18n::t("menu.settings"));
        items.push(i18n::t("menu.start_scheduler"));
        items.push(i18n::t("menu.exit"));

        println!("{}", ui::section(&i18n::t("menu.connections")));
        let item_refs: Vec<&str> = items.iter().map(String::as_str).collect();
        let sel = ui::select_default(&i18n::t("menu.connections"), &item_refs, 0)?;

        let settings_idx = items.len() - 3;
        let scheduler_idx = items.len() - 2;
        let exit_idx = items.len() - 1;

        match sel {
            0 => {
                if let Err(e) = add_connection(pool).await {
                    println!("{}", i18n::t_fmt("common.arrow_hint", &[("msg", &e.to_string())]));
                }
            }
            i if i == exit_idx => {
                println!("{}", i18n::t("menu.goodbye"));
                break;
            }
            i if i == scheduler_idx => {
                println!("{}", i18n::t("menu.scheduler_starting"));
                crate::scheduler::start_scheduler().await;
            }
            i if i == settings_idx => {
                if let Err(e) = settings_menu(pool).await {
                    println!("{}", i18n::t_fmt("common.arrow_hint", &[("msg", &e.to_string())]));
                }
            }
            i => connection_menu(pool, &connections[i - 1]).await?,
        }
    }
    Ok(())
}

async fn settings_menu(pool: &sqlx::SqlitePool) -> anyhow::Result<()> {
    loop {
        let custom = db::backups_root_custom(pool).await?;
        let folder_label = paths::backups_root_label(custom.as_deref());
        let lang_name = i18n::language_display_name(&i18n::current_language());

        let items = if custom.is_some() {
            vec![
                i18n::t_fmt("settings.language_current", &[("name", &lang_name)]),
                i18n::t_fmt("settings.backups_folder", &[("label", &folder_label)]),
                i18n::t("settings.reset_backups_folder"),
                i18n::t("settings.recreate_root_ca"),
                i18n::t("common.back"),
            ]
        } else {
            vec![
                i18n::t_fmt("settings.language_current", &[("name", &lang_name)]),
                i18n::t_fmt("settings.backups_folder", &[("label", &folder_label)]),
                i18n::t("settings.recreate_root_ca"),
                i18n::t("common.back"),
            ]
        };

        let item_refs: Vec<&str> = items.iter().map(String::as_str).collect();
        let sel = ui::select(&i18n::t("settings.title"), &item_refs)?;

        let back_idx = items.len() - 1;
        let recreate_idx = items.len() - 2;
        let reset_idx = if custom.is_some() {
            Some(items.len() - 3)
        } else {
            None
        };

        if sel == back_idx {
            return Ok(());
        }
        if sel == 0 {
            language_menu(pool).await?;
            continue;
        }
        if Some(sel) == reset_idx {
            db::reset_backups_root_default(pool).await?;
            let root = db::backups_root_path(pool).await?;
            paths::ensure_backups_root_exists(&root)?;
            ui::success_block(
                &i18n::t("settings.backups_reset_title"),
                &[&i18n::t_fmt(
                    "settings.backups_using_default",
                    &[("path", paths::DEFAULT_BACKUPS_DIR)],
                )],
            );
            continue;
        }
        if sel == recreate_idx {
            if let Err(e) = recreate_root_ca_menu(pool).await {
                println!("{}", i18n::t_fmt("common.arrow_hint", &[("msg", &e.to_string())]));
            }
            continue;
        }
        if sel == 1 {
            if let Err(e) = set_backups_folder_menu(pool).await {
                println!("{}", i18n::t_fmt("common.arrow_hint", &[("msg", &e.to_string())]));
            }
        }
    }
}

async fn language_menu(pool: &sqlx::SqlitePool) -> anyhow::Result<()> {
    let current = i18n::current_language();
    let items: Vec<String> = i18n::SUPPORTED
        .iter()
        .map(|(code, key)| {
            let name = i18n::t(key);
            if *code == current.as_str() {
                format!("{name} *")
            } else {
                name
            }
        })
        .collect();
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    let default = i18n::SUPPORTED
        .iter()
        .position(|(code, _)| *code == current.as_str())
        .unwrap_or(0);
    let sel = ui::select_default(&i18n::t("settings.language"), &refs, default)?;
    let (code, _) = i18n::SUPPORTED[sel];
    if code != current.as_str() {
        i18n::set_language(pool, code).await?;
    }
    Ok(())
}

async fn set_backups_folder_menu(pool: &sqlx::SqlitePool) -> anyhow::Result<()> {
    let default_root = paths::backups_root();
    let current = db::backups_root_custom(pool).await?;

    println!();
    println!("{}", i18n::t("settings.backups_intro_1"));
    println!("{}", i18n::t("settings.backups_intro_2"));
    println!();
    println!(
        "{}",
        i18n::t_fmt(
            "settings.backups_default",
            &[("path", paths::DEFAULT_BACKUPS_DIR)],
        )
    );
    if let Some(ref c) = current {
        println!(
            "{}",
            i18n::t_fmt("settings.backups_current", &[("path", c)])
        );
    }
    println!("{}", i18n::t("settings.backups_empty_default"));
    println!("{}", i18n::t("settings.backups_examples"));
    println!("{}", i18n::t("settings.backups_cloud"));
    println!("{}", i18n::t("settings.backups_path_ascii_hint"));
    println!();

    let default_input = current.unwrap_or_default();
    let raw = dialoguer::Input::<String>::new()
        .with_prompt(i18n::t("settings.backups_prompt"))
        .default(default_input)
        .allow_empty(true)
        .interact_text()?;
    let raw = raw.trim();

    if raw.is_empty() {
        db::reset_backups_root_default(pool).await?;
        paths::ensure_backups_root_exists(&default_root)?;
        ui::success_block(
            &i18n::t("settings.folder_title"),
            &[&i18n::t_fmt(
                "settings.backups_using_default",
                &[("path", paths::DEFAULT_BACKUPS_DIR)],
            )],
        );
        return Ok(());
    }

    let path = PathBuf::from(raw);
    let absolute = if path.is_absolute() {
        path
    } else {
        paths::app_root().join(path)
    };

    let absolute = absolute.canonicalize().unwrap_or(absolute);

    paths::validate_backups_root_writable(&absolute)?;

    if let Some(warn) = paths::backups_path_unicode_warning(absolute.to_string_lossy().as_ref()) {
        println!();
        println!("{}", warn);
    }

    db::set_backups_root_custom(pool, absolute.to_string_lossy().as_ref())
        .await?;

    ui::success_block(
        &i18n::t("settings.backups_saved_title"),
        &[
            &i18n::t_fmt(
                "settings.backups_saved_root",
                &[("path", &absolute.display().to_string())],
            ),
            &i18n::t("settings.backups_saved_files"),
            &i18n::t("settings.backups_saved_decrypt"),
        ],
    );
    Ok(())
}

async fn recreate_root_ca_menu(pool: &sqlx::SqlitePool) -> anyhow::Result<()> {
    let count = db::list_all_connections(pool).await?.len();
    println!();
    println!("{}", i18n::t("ca.recreate_intro_1"));
    println!("{}", i18n::t("ca.recreate_intro_2"));
    println!("{}", i18n::t("ca.recreate_intro_3"));
    println!("{}", i18n::t("ca.recreate_intro_4"));
    if count > 0 {
        println!(
            "{}",
            i18n::t_fmt("ca.recreate_intro_5", &[("count", &count.to_string())])
        );
    }
    println!();

    if !Confirm::new()
        .with_prompt(i18n::t("ca.recreate_confirm"))
        .default(false)
        .interact()?
    {
        println!("{}", i18n::t("common.cancelled_dot"));
        return Ok(());
    }

    ca::recreate_root_ca()?;
    ui::success_block(
        &i18n::t("ca.recreate_title"),
        &[
            &i18n::t_fmt(
                "ca.recreate_cert",
                &[("path", &ca::root_cert_path().display().to_string())],
            ),
            &i18n::t_fmt(
                "ca.recreate_key",
                &[("path", &ca::root_key_path().display().to_string())],
            ),
            &i18n::t("ca.recreate_next"),
        ],
    );
    if count > 0 {
        println!(
            "{}",
            i18n::t_fmt("ca.recreate_tls_count", &[("count", &count.to_string())])
        );
    }
    ui::press_enter();
    Ok(())
}

async fn add_connection(pool: &sqlx::SqlitePool) -> anyhow::Result<()> {
    let slug = prompt_slug(&i18n::t("connection.add_slug_prompt"), None)?;
    let server_addr = ui::prompt_server_addr(None)?;
    let url = ui::wss_url_from_server_addr(&server_addr)?;
    if db::connection_url_exists(pool, &url, None).await? {
        anyhow::bail!(
            "{}",
            i18n::t_fmt(
                "error.server_exists",
                &[("addr", &ui::server_addr_from_url(&url))],
            )
        );
    }

    println!("{}", i18n::t("connection.generating_tls"));
    let (url, fingerprint, _cert, agent_dir) =
        bundle::prepare_tls_connection(&slug, &server_addr)?;

    let id = db::add_connection(pool, &slug, &url, Some(&fingerprint)).await?;

    ui::success_block(
        &i18n::t("connection.added_title"),
        &[
            &i18n::t_fmt("connection.added_id", &[("slug", &slug), ("id", &id)]),
            &i18n::t_fmt("connection.added_ws", &[("url", &url)]),
            &i18n::t_fmt("connection.added_fp", &[("fingerprint", &fingerprint)]),
            &i18n::t_fmt(
                "connection.added_files",
                &[("path", &agent_dir.display().to_string())],
            ),
        ],
    );
    ui::print_vps_deploy_instructions(&agent_dir);
    println!("{}", i18n::t("connection.device_id_hint"));

    if Confirm::new()
        .with_prompt(i18n::t("connection.test_now"))
        .default(true)
        .interact()?
    {
        let conn = db::get_connection(pool, &id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("{}", i18n::t("connection.not_found")))?;
        test_connection(&conn, pool).await?;
        ui::press_enter();
    }

    Ok(())
}

async fn connection_menu(pool: &sqlx::SqlitePool, conn: &Connection) -> anyhow::Result<()> {
    let mut conn = conn.clone();
    loop {
        let tasks = db::list_tasks_for_connection(pool, &conn.id).await?;
        let conn_st = i18n::state_on_off(conn.enabled);
        println!(
            "{}",
            ui::done_line(
                &i18n::t("menu.connections"),
                &format!(
                    "{} | {} | {} | {} | retention {}",
                    ui::host_from_url(&conn.url),
                    conn.slug,
                    conn_st,
                    conn.secrets_mode_label(),
                    conn.retention_short()
                ),
            )
        );
        println!("{}", ui::section(&i18n::t("connection.tasks")));

        let mut items: Vec<String> = vec![i18n::t("connection.add_task")];
        for t in &tasks {
            let st = i18n::state_on_off(t.enabled);
            items.push(format!(
                "  {} | {} | {} | {}",
                t.task_name,
                task_params::display_task_type(&t.task_type),
                t.schedule,
                st
            ));
        }
        let secrets_detail = if conn.is_production() {
            i18n::t("connection.secrets_server")
        } else {
            i18n::t("connection.secrets_client")
        };
        items.push(i18n::t_fmt(
            "connection.secrets_mode",
            &[
                ("mode", &conn.secrets_mode_label()),
                ("detail", &secrets_detail),
            ],
        ));
        items.push(i18n::t_fmt(
            "connection.retention_line",
            &[("policy", &crate::retention::retention_label(conn.retention_days))],
        ));
        items.push(i18n::t("connection.test"));
        items.push(i18n::t_fmt(
            "connection.toggle_enabled",
            &[("state", &i18n::state_on_off(conn.enabled))],
        ));
        items.push(i18n::t("connection.renew_cert"));
        items.push(i18n::t("connection.edit"));
        items.push(i18n::t("connection.delete"));
        items.push(i18n::t("common.back"));

        let item_refs: Vec<&str> = items.iter().map(String::as_str).collect();
        let prompt = format!("{} | {}", ui::host_from_url(&conn.url), conn.slug);
        let sel = ui::select_default(&prompt, &item_refs, 0)?;

        let back_idx = items.len() - 1;
        let delete_idx = items.len() - 2;
        let edit_idx = items.len() - 3;
        let renew_idx = items.len() - 4;
        let toggle_idx = items.len() - 5;
        let test_idx = items.len() - 6;
        let retention_idx = items.len() - 7;
        let secrets_idx = items.len() - 8;

        match sel {
            0 => {
                if let Err(e) = add_task(pool, &conn).await {
                    println!("{}", i18n::t_fmt("common.arrow_hint", &[("msg", &e.to_string())]));
                }
            }
            i if i == back_idx => return Ok(()),
            i if i == delete_idx => {
                if Confirm::new()
                    .with_prompt(i18n::t("connection.delete_confirm"))
                    .default(false)
                    .interact()?
                {
                    db::delete_connection(pool, &conn.id).await?;
                    ui::success_block(&i18n::t("connection.deleted_title"), &[]);
                    ui::press_enter();
                    return Ok(());
                }
            }
            i if i == edit_idx => {
                if let Err(e) = edit_connection(pool, &conn).await {
                    println!("{}", i18n::t_fmt("common.arrow_hint", &[("msg", &e.to_string())]));
                } else if let Some(updated) = db::get_connection(pool, &conn.id).await? {
                    conn = updated;
                    ui::press_enter();
                }
            }
            i if i == renew_idx => {
                if let Err(e) = renew_server_cert(pool, &conn).await {
                    println!("{}", i18n::t_fmt("common.arrow_hint", &[("msg", &e.to_string())]));
                } else if let Some(updated) = db::get_connection(pool, &conn.id).await? {
                    conn = updated;
                }
            }
            i if i == toggle_idx => {
                conn.enabled = !conn.enabled;
                db::update_connection_enabled(pool, &conn.id, conn.enabled).await?;
                ui::success_block(
                    &i18n::t("connection.updated_title"),
                    &[&i18n::t_fmt(
                        "connection.enabled_line",
                        &[("state", &i18n::state_on_off(conn.enabled))],
                    )],
                );
            }
            i if i == secrets_idx => {
                if let Err(e) = toggle_secrets_mode(pool, &mut conn).await {
                    println!("{}", i18n::t_fmt("common.arrow_hint", &[("msg", &e.to_string())]));
                }
            }
            i if i == retention_idx => {
                if let Err(e) = configure_retention(pool, &mut conn).await {
                    println!("{}", i18n::t_fmt("common.arrow_hint", &[("msg", &e.to_string())]));
                }
            }
            i if i == test_idx => {
                if let Err(e) = test_connection(&conn, pool).await {
                    println!("{}", i18n::t_fmt("common.arrow_hint", &[("msg", &e.to_string())]));
                } else {
                    ui::press_enter();
                }
            }
            i if i - 1 < tasks.len() => task_menu(pool, &conn, &tasks[i - 1]).await?,
            _ => {}
        }
    }
}

async fn configure_retention(pool: &sqlx::SqlitePool, conn: &mut Connection) -> anyhow::Result<()> {
    let labels: Vec<String> = crate::retention::RETENTION_OPTIONS
        .iter()
        .map(|d| {
            if *d == 0 {
                i18n::t("retention.option_never")
            } else {
                let label = crate::retention::retention_label(*d);
                i18n::t_fmt("retention.option_days", &[("label", &label)])
            }
        })
        .collect();
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let default = crate::retention::RETENTION_OPTIONS
        .iter()
        .position(|d| *d == conn.retention_days)
        .unwrap_or(0);

    println!();
    println!("{}", i18n::t("retention.intro_1"));
    println!("{}", i18n::t("retention.intro_2"));
    println!("{}", i18n::t("retention.intro_3"));
    println!("{}", i18n::t("retention.intro_4"));
    println!("{}", i18n::t("retention.intro_5"));
    println!();

    let sel = ui::select_default(&i18n::t("retention.policy_prompt"), &refs, default)?;
    let days = crate::retention::RETENTION_OPTIONS[sel];

    db::update_connection_retention(pool, &conn.id, days).await?;
    conn.retention_days = days;

    ui::success_block(
        &i18n::t("retention.updated_title"),
        &[&i18n::t_fmt(
            "retention.updated_policy",
            &[("policy", &crate::retention::retention_label(days))],
        )],
    );
    ui::press_enter();
    Ok(())
}

async fn toggle_secrets_mode(
    pool: &sqlx::SqlitePool,
    conn: &mut Connection,
) -> anyhow::Result<()> {
    let switching_to_production = !conn.is_production();

    if switching_to_production {
        println!();
        println!("{}", i18n::t("secrets.prod_intro_1"));
        println!("{}", i18n::t("secrets.prod_intro_2"));
        println!("{}", i18n::t("secrets.prod_intro_3"));
        println!("{}", i18n::t("secrets.prod_intro_4"));
        println!("{}", i18n::t("secrets.prod_intro_5"));
        println!("{}", i18n::t("secrets.prod_intro_6"));
        println!();

        if !Confirm::new()
            .with_prompt(i18n::t("secrets.switch_prod"))
            .default(false)
            .interact()?
        {
            println!("{}", i18n::t("common.cancelled_dot"));
            return Ok(());
        }
    } else if !Confirm::new()
        .with_prompt(i18n::t("secrets.switch_test"))
        .default(false)
        .interact()?
    {
        println!("{}", i18n::t("common.cancelled_dot"));
        return Ok(());
    }

    let new_mode = if switching_to_production {
        "production"
    } else {
        "test"
    };
    db::update_connection_secrets_mode(pool, &conn.id, new_mode).await?;
    conn.secrets_mode = new_mode.to_string();

    if switching_to_production {
        ui::success_block(
            &i18n::t("secrets.prod_title"),
            &[&i18n::t("secrets.prod_l1"), &i18n::t("secrets.prod_l2")],
        );
    } else {
        ui::success_block(
            &i18n::t("secrets.test_title"),
            &[&i18n::t("secrets.test_l1")],
        );
    }
    ui::press_enter();
    Ok(())
}

async fn test_connection(conn: &Connection, pool: &sqlx::SqlitePool) -> anyhow::Result<()> {
    println!(
        "{}",
        i18n::t_fmt("connection.testing", &[("url", &conn.url)])
    );
    let (ws, _) = tls::connect_ws(conn).await?;
    let (write, mut read) = ws.split();
    let write = std::sync::Arc::new(tokio::sync::Mutex::new(write));
    protocol::authenticate_and_sync(conn, pool, &mut read, &write).await?;
    ui::success_block(
        &i18n::t("connection.ok_title"),
        &[&i18n::t("connection.ok_detail")],
    );
    crate::ws_inbound::graceful_close(&mut read, &write).await;
    Ok(())
}

async fn renew_server_cert(pool: &sqlx::SqlitePool, conn: &Connection) -> anyhow::Result<()> {
    let server_addr = ui::server_addr_from_url(&conn.url);
    println!();
    println!(
        "{}",
        i18n::t_fmt(
            "connection.renew_intro",
            &[("slug", &conn.slug), ("addr", &server_addr)],
        )
    );
    println!("{}", i18n::t("connection.renew_b1"));
    println!("{}", i18n::t("connection.renew_b2"));
    println!("{}", i18n::t("connection.renew_b3"));
    println!();

    if !Confirm::new()
        .with_prompt(i18n::t("connection.renew_confirm"))
        .default(false)
        .interact()?
    {
        println!("{}", i18n::t("common.cancelled_dot"));
        return Ok(());
    }

    println!("{}", i18n::t("connection.renew_issuing"));
    let (url, fingerprint, _cert, agent_dir) =
        bundle::prepare_tls_connection(&conn.slug, &server_addr)?;

    db::update_connection_tls(pool, &conn.id, &url, &fingerprint).await?;

    ui::success_block(
        &i18n::t("connection.renew_title"),
        &[
            &i18n::t_fmt("connection.renew_conn", &[("slug", &conn.slug)]),
            &i18n::t_fmt("connection.renew_fp", &[("fingerprint", &fingerprint)]),
            &i18n::t_fmt(
                "connection.renew_folder",
                &[("path", &agent_dir.display().to_string())],
            ),
            &i18n::t("connection.renew_device_id"),
        ],
    );
    ui::print_vps_deploy_instructions(&agent_dir);
    println!("{}", i18n::t("connection.renew_step1"));
    println!("{}", i18n::t("connection.renew_step2"));
    ui::press_enter();
    Ok(())
}

async fn edit_connection(pool: &sqlx::SqlitePool, conn: &Connection) -> anyhow::Result<()> {
    let slug = prompt_slug(&i18n::t("connection.edit_slug"), Some(&conn.slug))?;
    let server_addr = ui::prompt_server_addr(Some(&ui::server_addr_from_url(&conn.url)))?;
    let url = ui::wss_url_from_server_addr(&server_addr)?;
    if db::connection_url_exists(pool, &url, Some(&conn.id)).await? {
        anyhow::bail!(
            "{}",
            i18n::t_fmt(
                "error.server_used_other",
                &[("addr", &ui::server_addr_from_url(&url))],
            )
        );
    }
    db::update_connection(
        pool,
        &conn.id,
        &slug,
        &url,
        conn.cert_fingerprint.as_deref(),
    )
    .await?;
    ui::success_block(&i18n::t("connection.updated_title"), &[]);
    Ok(())
}

async fn add_task(pool: &sqlx::SqlitePool, conn: &Connection) -> anyhow::Result<()> {
    let (task_name, mut task_type, data_json, schedule, enabled) =
        match task_params::prompt_new_task().await {
            Ok(v) => v,
            Err(e) if e.to_string() == task_params::CANCELLED => return Ok(()),
            Err(e) => {
                println!("{}", i18n::t_fmt("common.arrow_hint", &[("msg", &e.to_string())]));
                return Ok(());
            }
        };
    task_type = crate::validation::normalize_task_type(&task_type).to_string();
    let id = format!("task_{}", uuid::Uuid::new_v4().simple());
    db::add_task(
        pool, &id, &conn.id, &task_name, &task_type, &data_json, &schedule, enabled,
    )
    .await?;
    ui::success_block(
        &i18n::t("task.added_title"),
        &[&i18n::t_fmt("task.added_name", &[("name", &task_name)])],
    );
    Ok(())
}

async fn task_menu(pool: &sqlx::SqlitePool, conn: &Connection, task: &Task) -> anyhow::Result<()> {
    #[derive(Copy, Clone, Eq, PartialEq)]
    enum Choice {
        EditName,
        EditType,
        EditParams,
        EditSchedule,
        ToggleEnabled,
        EncryptMode,
        DirSyncLastFile,
        RunNow,
        Delete,
        Back,
    }

    let mut current = task.clone();

    loop {
        let dir_sync_task =
            crate::validation::normalize_task_type(&current.task_type) == "dir_sync";
        println!(
            "{}",
            ui::done_line(
                &i18n::t("task.title"),
                &format!(
                    "{} | {} | {} | {} | encrypt {}",
                    current.task_name,
                    task_params::display_task_type(&current.task_type),
                    current.schedule,
                    i18n::state_on_off(current.enabled),
                    task_params::encrypt_display(&current.task_type, &current.data_json),
                ),
            )
        );

        let params_summary = task_params::params_summary(&current.task_type, &current.data_json);
        let edit_params = if params_summary.is_empty() {
            i18n::t("task.edit_params")
        } else {
            i18n::t_fmt("task.edit_params_summary", &[("summary", &params_summary)])
        };

        let mut actions: Vec<Choice> = vec![
            Choice::EditName,
            Choice::EditType,
            Choice::EditParams,
            Choice::EditSchedule,
            Choice::ToggleEnabled,
            Choice::EncryptMode,
        ];

        if dir_sync_task {
            actions.push(Choice::DirSyncLastFile);
        }

        actions.push(Choice::RunNow);
        actions.push(Choice::Delete);
        actions.push(Choice::Back);

        let mut items: Vec<String> = vec![
            i18n::t_fmt("task.edit_name", &[("name", &current.task_name)]),
            i18n::t_fmt(
                "task.edit_type",
                &[(
                    "type",
                    task_params::display_task_type(&current.task_type),
                )],
            ),
            edit_params,
            i18n::t_fmt("task.edit_schedule", &[("schedule", &current.schedule)]),
            i18n::t_fmt(
                "task.toggle_enabled",
                &[("state", &i18n::state_on_off(current.enabled))],
            ),
            i18n::t_fmt(
                "task.encrypt_mode",
                &[(
                    "state",
                    &task_params::encrypt_display(&current.task_type, &current.data_json),
                )],
            ),
        ];

        if dir_sync_task {
            let last_file = crate::dir_sync::last_received_filename(pool, conn, &current).await?;
            let label = match last_file {
                Some(name) => i18n::t_fmt("task.dir_sync_last_file", &[("filename", &name)]),
                None => i18n::t("task.dir_sync_last_file_none"),
            };
            items.push(label);
        }

        items.push(i18n::t("task.run_now"));
        items.push(i18n::t("task.delete"));
        items.push(i18n::t("common.back"));

        let item_refs: Vec<&str> = items.iter().map(String::as_str).collect();
        let prompt = i18n::t_fmt(
            "task.header",
            &[
                ("name", &current.task_name),
                (
                    "type",
                    task_params::display_task_type(&current.task_type),
                ),
            ],
        );
        let sel = ui::select(&prompt, &item_refs)?;
        let choice = actions[sel];

        match choice {
            Choice::EditName => {
                let task_name = prompt_slug(
                    &i18n::t("task.name_prompt"),
                    Some(&current.task_name),
                )?;
                db::update_task(
                    pool,
                    &current.id,
                    &task_name,
                    &current.task_type,
                    &current.data_json,
                    &current.schedule,
                    current.enabled,
                )
                .await?;
                current.task_name = task_name;
                ui::success_block(&i18n::t("common.updated"), &[]);
            }
            Choice::EditType => {
                if let Some(task_type) = task_params::prompt_task_type().await? {
                    let task_type = crate::validation::normalize_task_type(&task_type).to_string();
                    let change_data = Confirm::new()
                        .with_prompt(i18n::t("task.reenter_params"))
                        .default(true)
                        .interact()?;
                    let data_json = if change_data {
                        task_params::prompt_task_data(&task_type, None).await?
                    } else {
                        current.data_json.clone()
                    };
                    let encrypt = if crate::validation::normalize_task_type(&task_type) == "dir_sync"
                    {
                        false
                    } else if change_data {
                        task_params::prompt_encrypt_mode(
                            &task_type,
                            task_params::read_encrypt_flag(&current.data_json),
                        )?
                    } else {
                        task_params::read_encrypt_flag(&data_json)
                    };
                    let data_json =
                        task_params::merge_encrypt_flag(&task_type, &data_json, encrypt)?;
                    db::update_task(
                        pool,
                        &current.id,
                        &current.task_name,
                        &task_type,
                        &data_json,
                        &current.schedule,
                        current.enabled,
                    )
                    .await?;
                    current.task_type = task_type;
                    current.data_json = data_json;
                    ui::success_block(&i18n::t("common.updated"), &[]);
                }
            }
            Choice::EditParams => {
                let encrypt = task_params::read_encrypt_flag(&current.data_json);
                let data_json =
                    task_params::prompt_task_data(&current.task_type, Some(&current.data_json))
                        .await?;
                let data_json =
                    task_params::merge_encrypt_flag(&current.task_type, &data_json, encrypt)?;
                db::update_task(
                    pool,
                    &current.id,
                    &current.task_name,
                    &current.task_type,
                    &data_json,
                    &current.schedule,
                    current.enabled,
                )
                .await?;
                current.data_json = data_json;
                ui::success_block(&i18n::t("common.updated"), &[]);
            }
            Choice::EditSchedule => {
                let schedule = task_params::prompt_schedule(Some(&current.schedule))?;
                db::update_task(
                    pool,
                    &current.id,
                    &current.task_name,
                    &current.task_type,
                    &current.data_json,
                    &schedule,
                    current.enabled,
                )
                .await?;
                current.schedule = schedule;
                ui::success_block(&i18n::t("common.updated"), &[]);
            }
            Choice::ToggleEnabled => {
                current.enabled = !current.enabled;
                db::update_task_enabled(pool, &current.id, current.enabled).await?;
            }
            Choice::EncryptMode => {
                if crate::validation::normalize_task_type(&current.task_type) == "dir_sync" {
                    println!("{}", i18n::t("task.dir_sync_no_encrypt"));
                    continue;
                }
                let encrypt = task_params::prompt_encrypt_mode(
                    &current.task_type,
                    task_params::read_encrypt_flag(&current.data_json),
                )?;
                let data_json =
                    task_params::merge_encrypt_flag(&current.task_type, &current.data_json, encrypt)?;
                db::update_task(
                    pool,
                    &current.id,
                    &current.task_name,
                    &current.task_type,
                    &data_json,
                    &current.schedule,
                    current.enabled,
                )
                .await?;
                current.data_json = data_json;
                ui::success_block(&i18n::t("common.updated"), &[]);
            }
            Choice::DirSyncLastFile => {
                if Confirm::new()
                    .with_prompt(i18n::t("task.dir_sync_reset_confirm"))
                    .default(false)
                    .interact()?
                {
                    println!("{}", i18n::t("task.dir_sync_resetting"));
                    if let Err(e) = runner::reset_dir_sync_now(conn, &current).await {
                        println!(
                            "{}",
                            i18n::t_fmt(
                                "task.dir_sync_reset_failed",
                                &[("err", &e.to_string())],
                            )
                        );
                    } else {
                        ui::success_block(&i18n::t("task.dir_sync_reset_ok"), &[]);
                    }
                }
            }
            Choice::RunNow => {
                println!("{}", i18n::t("task.running"));
                if let Err(e) = runner::run_task_now(conn, &current).await {
                    println!(
                        "{}",
                        i18n::t_fmt("task.run_failed", &[("err", &e.to_string())])
                    );
                } else {
                    ui::success_block(&i18n::t("task.completed_title"), &[]);
                    ui::press_enter();
                }
            }
            Choice::Delete => {
                if Confirm::new()
                    .with_prompt(i18n::t("task.delete_confirm"))
                    .default(false)
                    .interact()?
                {
                    db::delete_task(pool, &current.id).await?;
                    ui::success_block(&i18n::t("task.deleted_title"), &[]);
                    return Ok(());
                }
            }
            Choice::Back => return Ok(()),
        }
    }
}
