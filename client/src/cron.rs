//! Cron schedule normalization (5-field to 6-field for tokio-cron-scheduler).

use chrono::{DateTime, Local};
use std::str::FromStr;
use std::time::Duration;

use crate::i18n;

pub fn schedule_help() -> String {
    i18n::t("schedule.help")
}

fn fix_field(part: &str) -> String {
    let part = part.trim();
    if part.is_empty() {
        return "*".to_string();
    }
    if part.starts_with('/') && !part.starts_with("*/") {
        format!("*/{}", part.trim_start_matches('/'))
    } else {
        part.to_string()
    }
}

/// Minute-step value from `*/N` or `/N` (before padding other fields).
fn parse_step_minutes(field: &str) -> Option<u32> {
    let field = field.trim();
    let n_str = field
        .strip_prefix("*/")
        .or_else(|| field.strip_prefix('/'))?;
    n_str.parse().ok()
}

/// `*/60`, `*/120`, … → hourly cron when remaining fields are wildcards.
/// Standard cron minutes are 0–59; intervals ≥60 minutes belong in the hour field.
fn try_convert_minute_interval_to_hours(parts: &mut [String]) -> bool {
    if parts.len() < 5 || !parts[1..].iter().all(|p| p == "*") {
        return false;
    }
    let Some(minutes) = parse_step_minutes(&parts[0]) else {
        return false;
    };
    if minutes < 60 || minutes % 60 != 0 {
        return false;
    }
    let hours = minutes / 60;
    parts[0] = "0".to_string();
    parts[1] = if hours == 1 {
        "*".to_string()
    } else {
        format!("*/{hours}")
    };
    true
}

pub fn normalize_schedule(schedule: &str) -> String {
    let schedule = schedule.trim();
    if schedule.is_empty() {
        return String::new();
    }

    let mut parts: Vec<String> = schedule
        .split_whitespace()
        .map(|p| fix_field(p))
        .filter(|p| !p.is_empty())
        .collect();

    while parts.len() < 5 {
        parts.push("*".to_string());
    }

    if parts.len() > 5 {
        parts.truncate(5);
    }

    let _ = try_convert_minute_interval_to_hours(&mut parts);

    parts.join(" ")
}

pub fn to_six_field(schedule: &str) -> anyhow::Result<String> {
    let schedule = normalize_schedule(schedule);
    let parts: Vec<&str> = schedule.split_whitespace().collect();
    match parts.len() {
        5 => Ok(format!("0 {}", parts.join(" "))),
        6 => Ok(parts.join(" ")),
        n => anyhow::bail!(
            "{}",
            i18n::t_fmt(
                "error.schedule_invalid",
                &[("count", &n.to_string()), ("expr", &schedule)]
            )
        ),
    }
}

fn is_all_wildcards(schedule: &str) -> bool {
    schedule
        .split_whitespace()
        .all(|p| p == "*")
}

pub fn validate_schedule(schedule: &str) -> anyhow::Result<String> {
    let raw = schedule.trim();
    if raw.is_empty() {
        anyhow::bail!("{}", i18n::t("error.schedule_required"));
    }

    let field_count = raw.split_whitespace().count();
    if field_count > 5 {
        anyhow::bail!(
            "{}",
            i18n::t_fmt(
                "error.schedule_too_many",
                &[("count", &field_count.to_string())]
            )
        );
    }

    let normalized = normalize_schedule(raw);

    if is_all_wildcards(&normalized) {
        anyhow::bail!("{}", i18n::t("error.schedule_too_broad"));
    }

    if let Some(minutes) = normalized.split_whitespace().next().and_then(parse_step_minutes) {
        if minutes > 59 {
            anyhow::bail!("{}", i18n::t("error.schedule_minute_interval"));
        }
    }

    to_six_field(&normalized)?;
    Ok(normalized)
}

/// Next fire time strictly after `after` (local timezone).
pub fn next_run_after(schedule: &str, after: DateTime<Local>) -> anyhow::Result<Option<DateTime<Local>>> {
    let six = to_six_field(schedule)?;
    let sched = cron::Schedule::from_str(&six)
        .map_err(|e| anyhow::anyhow!("invalid cron expression: {e}"))?;
    Ok(sched.after(&after).next())
}

/// True if any enabled task on this connection fires within `within` from now.
pub async fn next_task_within_for_connection(
    pool: &sqlx::SqlitePool,
    connection_id: &str,
    within: Duration,
) -> anyhow::Result<bool> {
    let now = Local::now();
    let deadline = now
        + chrono::Duration::from_std(within)
            .map_err(|e| anyhow::anyhow!("invalid duration: {e}"))?;
    let tasks = crate::db::list_tasks_for_connection(pool, connection_id).await?;
    for task in tasks.iter().filter(|t| t.enabled) {
        if let Some(next) = next_run_after(&task.schedule, now)? {
            if next <= deadline {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// True if any enabled task on any connection sharing `url` fires within `within`.
pub async fn next_task_within_for_url(
    pool: &sqlx::SqlitePool,
    url: &str,
    within: Duration,
) -> anyhow::Result<bool> {
    let conns = crate::db::list_enabled_connections(pool).await?;
    for conn in conns.iter().filter(|c| c.url == url) {
        if next_task_within_for_connection(pool, &conn.id, within).await? {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn next_run_finds_following_slot() {
        let after = Local.with_ymd_and_hms(2026, 7, 4, 22, 0, 0).unwrap();
        let next = next_run_after("5 22 * * *", after).unwrap().unwrap();
        assert_eq!(next, Local.with_ymd_and_hms(2026, 7, 4, 22, 5, 0).unwrap());
    }

    #[test]
    fn converts_five_field_cron() {
        assert_eq!(to_six_field("0 8 * * *").unwrap(), "0 0 8 * * *");
    }

    #[test]
    fn fixes_slash_one_typo() {
        assert_eq!(
            validate_schedule("/1 * * * *").unwrap(),
            "*/1 * * * *"
        );
    }

    #[test]
    fn pads_short_input() {
        assert_eq!(validate_schedule("*/1").unwrap(), "*/1 * * * *");
        assert_eq!(validate_schedule("0 8").unwrap(), "0 8 * * *");
        assert_eq!(validate_schedule("/1").unwrap(), "*/1 * * * *");
    }

    #[test]
    fn rejects_all_wildcards() {
        assert!(validate_schedule("* * * * *").is_err());
        assert!(validate_schedule("*").is_err());
    }

    #[test]
    fn rejects_empty() {
        assert!(validate_schedule("").is_err());
        assert!(validate_schedule("   ").is_err());
    }

    #[test]
    fn converts_minute_interval_shorthand() {
        assert_eq!(validate_schedule("*/60").unwrap(), "0 * * * *");
        assert_eq!(validate_schedule("*/120").unwrap(), "0 */2 * * *");
        assert_eq!(validate_schedule("/60").unwrap(), "0 * * * *");
        assert_eq!(validate_schedule("*/180 * * * *").unwrap(), "0 */3 * * *");
    }

    #[test]
    fn rejects_non_hour_minute_interval() {
        assert!(validate_schedule("*/90").is_err());
    }
}
