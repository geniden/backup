//! Input validation: slugs, task types, DB fields.

use crate::i18n;

pub fn sanitize_slug(input: &str) -> String {
    input
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect::<String>()
}

pub fn is_latin_identifier(input: &str) -> bool {
    let input = input.trim();
    !input.is_empty()
        && input
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub fn validate_db_identifier(label: &str, value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(i18n::t_fmt("validation.field_required", &[("label", label)]));
    }
    if !is_latin_identifier(value) {
        return Err(i18n::t_fmt("validation.field_latin", &[("label", label)]));
    }
    Ok(value.to_string())
}

pub fn validate_db_host(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(i18n::t("validation.db_host_required"));
    }

    if value == "localhost" {
        return Ok(value.to_string());
    }

    if value
        .chars()
        .all(|c| c.is_ascii_digit() || c == '.')
        && value.parse::<std::net::IpAddr>().is_ok()
    {
        return Ok(value.to_string());
    }

    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return Ok(value.to_string());
    }

    Err(i18n::t("validation.db_host_format"))
}

pub fn validate_db_port(value: &str) -> Result<u16, String> {
    let value = value.trim();
    let port: u16 = value
        .parse()
        .map_err(|_| i18n::t("validation.db_port_number"))?;
    if port == 0 {
        return Err(i18n::t("validation.db_port_range"));
    }
    Ok(port)
}

pub fn prompt_slug(prompt: &str, default: Option<&str>) -> anyhow::Result<String> {
    loop {
        let raw = if let Some(d) = default {
            dialoguer::Input::<String>::new()
                .with_prompt(prompt)
                .default(d.to_string())
                .interact_text()?
        } else {
            dialoguer::Input::<String>::new()
                .with_prompt(prompt)
                .interact_text()?
        };

        let slug = sanitize_slug(&raw);
        if slug.is_empty() {
            println!("{}", i18n::t("validation.slug_empty"));
            continue;
        }
        if slug != raw.trim() {
            println!(
                "{}",
                i18n::t_fmt("validation.slug_normalized", &[("slug", &slug)])
            );
        }
        return Ok(slug);
    }
}

fn prompt_validated(
    prompt: &str,
    default: Option<&str>,
    validate: impl Fn(&str) -> Result<String, String>,
) -> anyhow::Result<String> {
    loop {
        let raw = if let Some(d) = default {
            dialoguer::Input::<String>::new()
                .with_prompt(prompt)
                .default(d.to_string())
                .interact_text()?
        } else {
            dialoguer::Input::<String>::new()
                .with_prompt(prompt)
                .interact_text()?
        };

        match validate(&raw) {
            Ok(value) => return Ok(value),
            Err(msg) => println!("{}", i18n::t_fmt("common.arrow_hint", &[("msg", &msg)])),
        }
    }
}

pub fn prompt_db_name(default: Option<&str>) -> anyhow::Result<String> {
    let label = i18n::t("validation.label.db_name");
    prompt_validated(
        &i18n::t("validation.db_name_prompt"),
        default,
        move |v| validate_db_identifier(&label, v),
    )
}

pub fn prompt_db_user(default: Option<&str>) -> anyhow::Result<String> {
    let label = i18n::t("validation.label.db_user");
    prompt_validated(
        &i18n::t("validation.db_user_prompt"),
        default,
        move |v| validate_db_identifier(&label, v),
    )
}

pub fn prompt_db_host(default: &str) -> anyhow::Result<String> {
    prompt_validated(
        &i18n::t("validation.db_host_prompt"),
        Some(default),
        validate_db_host,
    )
}

pub fn prompt_db_port(default: &str) -> anyhow::Result<u16> {
    loop {
        let raw = dialoguer::Input::<String>::new()
            .with_prompt(i18n::t("validation.db_port_prompt"))
            .default(default.to_string())
            .interact_text()?;

        match validate_db_port(&raw) {
            Ok(port) => return Ok(port),
            Err(msg) => println!("{}", i18n::t_fmt("common.arrow_hint", &[("msg", &msg)])),
        }
    }
}

pub fn normalize_task_type(task_type: &str) -> &str {
    match task_type {
        "file_archive" => "files_archive",
        other => other,
    }
}
