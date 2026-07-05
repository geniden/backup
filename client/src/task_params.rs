//! Interactive prompts for task type parameters.

use crate::cron;
use crate::i18n;
use crate::models::shell::ShellParams;
use crate::validation::{
    normalize_task_type, prompt_db_host, prompt_db_name, prompt_db_port, prompt_db_user,
    prompt_slug,
};

pub const TASK_TYPES: &[&str] = &[
    "mysql_dump",
    "postgresql_dump",
    "sqlite_dump",
    "files_archive",
    "dir_sync",
    "shell",
];

/// Internal cancellation marker (locale-independent).
pub const CANCELLED: &str = "__cancelled__";

pub fn display_task_type(t: &str) -> &str {
    normalize_task_type(t)
}

pub async fn prompt_task_type() -> anyhow::Result<Option<String>> {
    let mut types: Vec<String> = TASK_TYPES.iter().map(|s| s.to_string()).collect();
    types.push(i18n::t("common.back"));

    let sel = dialoguer::Select::new()
        .with_prompt(i18n::t("task.type_prompt"))
        .items(&types)
        .interact()?;

    if types[sel] == i18n::t("common.back") {
        return Ok(None);
    }
    Ok(Some(types[sel].clone()))
}

pub fn params_summary(task_type: &str, data_json: &str) -> String {
    let task_type = normalize_task_type(task_type);
    let Ok(data) = serde_json::from_str::<serde_json::Value>(data_json) else {
        return String::new();
    };
    let em = i18n::t("task.params_em_dash");

    match task_type {
        "files_archive" => {
            let path = data
                .get("source_path")
                .or_else(|| data.get("source"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(&em);
            let ignore = ignore_patterns_display(&data);
            i18n::t_fmt(
                "task.params_path",
                &[("path", path), ("ignore", &ignore)],
            )
        }
        "dir_sync" => {
            let path = data
                .get("source_path")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(&em);
            let days = data
                .get("first_sync_days")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                .to_string();
            let batch = data
                .get("max_batch_mb")
                .and_then(|v| v.as_u64())
                .unwrap_or(200)
                .to_string();
            i18n::t_fmt(
                "task.params_dir_sync",
                &[("path", path), ("days", &days), ("batch", &batch)],
            )
        }
        _ => String::new(),
    }
}

pub async fn prompt_task_data(task_type: &str, existing: Option<&str>) -> anyhow::Result<String> {
    let task_type = normalize_task_type(task_type);
    let existing_data = existing
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .unwrap_or(serde_json::Value::Null);
    let em = i18n::t("task.params_em_dash");

    let data = match task_type {
        "mysql_dump" | "postgresql_dump" => prompt_db_dump_params(task_type)?,
        "sqlite_dump" => {
            let default_path = existing_data
                .get("db_path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let path = dialoguer::Input::<String>::new()
                .with_prompt(i18n::t("task.param.db_file"))
                .default(default_path)
                .interact_text()?;
            serde_json::json!({ "db_path": path })
        }
        "files_archive" => {
            let default_source = existing_data
                .get("source_path")
                .or_else(|| existing_data.get("source"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let default_ignore = ignore_patterns_display(&existing_data);
            let default_ignore = if default_ignore == em {
                String::new()
            } else {
                default_ignore
            };

            let source = dialoguer::Input::<String>::new()
                .with_prompt(i18n::t("task.param.source_dir"))
                .default(default_source)
                .interact_text()?;
            let ignore_str = dialoguer::Input::<String>::new()
                .with_prompt(i18n::t("task.param.ignore"))
                .default(default_ignore)
                .interact_text()?;
            let ignore: Vec<String> = ignore_str
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            serde_json::json!({ "source_path": source, "ignore": ignore })
        }
        "dir_sync" => {
            let default_source = existing_data
                .get("source_path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let default_days = existing_data
                .get("first_sync_days")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                .to_string();

            println!();
            println!("{}", i18n::t("task.dir_sync_intro_1"));
            println!("{}", i18n::t("task.dir_sync_intro_2"));
            println!("{}", i18n::t("task.dir_sync_intro_3"));
            println!("{}", i18n::t("task.dir_sync_intro_4"));
            println!();

            let source = dialoguer::Input::<String>::new()
                .with_prompt(i18n::t("task.param.source_server"))
                .default(default_source)
                .interact_text()?;
            let days_str = dialoguer::Input::<String>::new()
                .with_prompt(i18n::t("task.param.first_sync"))
                .default(default_days)
                .interact_text()?;
            let first_sync_days: u64 = days_str.parse().unwrap_or(0);

            let default_batch = existing_data
                .get("max_batch_mb")
                .and_then(|v| v.as_u64())
                .unwrap_or(200)
                .to_string();
            println!();
            println!("{}", i18n::t("task.batch_intro_1"));
            println!("{}", i18n::t("task.batch_intro_2"));
            let batch_str = dialoguer::Input::<String>::new()
                .with_prompt(i18n::t("task.param.max_batch"))
                .default(default_batch)
                .interact_text()?;
            let parsed = batch_str.parse::<u64>().unwrap_or(200);
            let max_batch_mb = parsed.clamp(1, 500);
            if parsed > 500 {
                println!("{}", i18n::t("task.batch_capped"));
            } else if parsed > 200 {
                println!("{}", i18n::t("task.batch_note"));
            }

            serde_json::json!({
                "source_path": source,
                "first_sync_days": first_sync_days,
                "max_batch_mb": max_batch_mb
            })
        }
        "shell" => {
            let p = prompt_shell_params(Some(&existing_data))?;
            serde_json::to_value(p)?
        }
        _ => serde_json::json!({}),
    };
    Ok(serde_json::to_string(&data)?)
}

pub fn encrypt_display(task_type: &str, data_json: &str) -> String {
    if normalize_task_type(task_type) == "dir_sync" {
        return i18n::encrypt_state(false, true);
    }
    i18n::encrypt_state(read_encrypt_flag(data_json), false)
}

pub fn read_encrypt_flag(data_json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(data_json)
        .ok()
        .and_then(|v| v.get("encrypt").and_then(|b| b.as_bool()))
        .unwrap_or(false)
}

pub fn merge_encrypt_flag(task_type: &str, data_json: &str, encrypt: bool) -> anyhow::Result<String> {
    let task_type = normalize_task_type(task_type);
    let mut data: serde_json::Value = serde_json::from_str(data_json)?;
    let obj = data
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("Task data must be a JSON object"))?;
    let encrypt = if task_type == "dir_sync" { false } else { encrypt };
    obj.insert("encrypt".to_string(), serde_json::Value::Bool(encrypt));
    Ok(serde_json::to_string(obj)?)
}

pub fn prompt_encrypt_mode(task_type: &str, current: bool) -> anyhow::Result<bool> {
    let task_type = normalize_task_type(task_type);
    if task_type == "dir_sync" {
        return Ok(false);
    }
    let encrypt = dialoguer::Confirm::new()
        .with_prompt(i18n::t("task.encrypt_prompt"))
        .default(current)
        .interact()?;
    if encrypt {
        println!();
        println!("{}", i18n::t("task.encrypt_hint_1"));
        println!("{}", i18n::t("task.encrypt_hint_2"));
        println!("{}", i18n::t("task.encrypt_hint_3"));
        println!();
    }
    Ok(encrypt)
}

pub async fn prompt_new_task() -> anyhow::Result<(String, String, String, String, bool)> {
    let task_type = prompt_task_type()
        .await?
        .ok_or_else(|| anyhow::anyhow!(CANCELLED))?;
    let task_name = prompt_slug(&i18n::t("task.name_prompt"), None)?;
    let data_json = prompt_task_data(&task_type, None).await?;
    let encrypt = prompt_encrypt_mode(&task_type, false)?;
    let data_json = merge_encrypt_flag(&task_type, &data_json, encrypt)?;
    let schedule = prompt_schedule(None)?;
    let enabled = dialoguer::Confirm::new()
        .with_prompt(i18n::t("task.enable_confirm"))
        .default(true)
        .interact()?;
    Ok((task_name, task_type, data_json, schedule, enabled))
}

pub fn prompt_schedule(default: Option<&str>) -> anyhow::Result<String> {
    println!("{}", cron::schedule_help());
    let default = default
        .map(cron::normalize_schedule)
        .unwrap_or_else(|| "0 8 * * *".to_string());
    loop {
        let raw = dialoguer::Input::<String>::new()
            .with_prompt(i18n::t("schedule.prompt"))
            .default(default.clone())
            .interact_text()?;
        match cron::validate_schedule(&raw) {
            Ok(normalized) => {
                if normalized != raw.trim() {
                    println!(
                        "{}",
                        i18n::t_fmt("schedule.saved_as", &[("expr", &normalized)])
                    );
                }
                return Ok(normalized);
            }
            Err(e) => println!("{}", i18n::t_fmt("common.arrow_hint", &[("msg", &e.to_string())])),
        }
    }
}

fn prompt_db_dump_params(task_type: &str) -> anyhow::Result<serde_json::Value> {
    let db_name = prompt_db_name(None)?;
    let db_user = prompt_db_user(None)?;
    let db_pass = dialoguer::Input::<String>::new()
        .with_prompt(i18n::t("task.param.db_password"))
        .interact()?;

    let (default_host, default_port) = if task_type == "postgresql_dump" {
        ("localhost", "5432")
    } else {
        ("127.0.0.1", "3306")
    };

    let db_host = prompt_db_host(default_host)?;
    let db_port = prompt_db_port(default_port)?;

    Ok(serde_json::json!({
        "db_name": db_name,
        "db_user": db_user,
        "db_pass": db_pass,
        "db_host": db_host,
        "db_port": db_port,
        "provider": if task_type == "postgresql_dump" { "postgresql" } else { "mysql" },
    }))
}

fn prompt_shell_params(existing: Option<&serde_json::Value>) -> anyhow::Result<ShellParams> {
    let existing = existing.cloned().unwrap_or(serde_json::Value::Null);
    let default_script = existing
        .get("script_name")
        .or_else(|| existing.get("script"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let default_args = existing
        .get("script_args")
        .or_else(|| existing.get("args"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let default_timeout = existing
        .get("timeout_secs")
        .or_else(|| existing.get("timeout"))
        .map(|v| v.to_string())
        .unwrap_or_else(|| "60".to_string());

    let script = dialoguer::Input::<String>::new()
        .with_prompt(i18n::t("task.param.script"))
        .default(default_script)
        .interact_text()?;
    let args_str = dialoguer::Input::<String>::new()
        .with_prompt(i18n::t("task.param.args"))
        .default(default_args)
        .interact_text()?;
    let timeout_str = dialoguer::Input::<String>::new()
        .with_prompt(i18n::t("task.param.timeout"))
        .default(default_timeout)
        .interact_text()?;

    Ok(ShellParams {
        script,
        args: args_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        timeout_secs: timeout_str.parse().unwrap_or(60),
    })
}

fn ignore_patterns_display(data: &serde_json::Value) -> String {
    let em = i18n::t("task.params_em_dash");
    if let Some(arr) = data.get("ignore").and_then(|v| v.as_array()) {
        let items: Vec<_> = arr
            .iter()
            .filter_map(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .collect();
        if items.is_empty() {
            return em;
        }
        return items.join(", ");
    }
    if let Some(s) = data.get("ignore").and_then(|v| v.as_str()) {
        if s.is_empty() {
            em
        } else {
            s.to_string()
        }
    } else {
        em
    }
}
