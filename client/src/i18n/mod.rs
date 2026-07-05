//! UI localization: strings embedded at compile time via `include_str!` (no runtime folder).

use std::collections::HashMap;
use std::sync::RwLock;

use once_cell::sync::Lazy;
use sqlx::SqlitePool;

const EN_JSON: &str = include_str!("en.json");
const RU_JSON: &str = include_str!("ru.json");
const DE_JSON: &str = include_str!("de.json");
const FR_JSON: &str = include_str!("fr.json");
const ZH_CN_JSON: &str = include_str!("zh-CN.json");

pub const SETTING_LANGUAGE: &str = "language";
pub const DEFAULT_LANGUAGE: &str = "en";

pub static SUPPORTED: &[(&str, &str)] = &[
    ("en", "lang.en"),
    ("ru", "lang.ru"),
    ("de", "lang.de"),
    ("fr", "lang.fr"),
    ("zh-cn", "lang.zh_cn"),
];

static CATALOG: Lazy<HashMap<String, HashMap<String, String>>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert("en".to_string(), parse_locale(EN_JSON));
    m.insert("ru".to_string(), parse_locale(RU_JSON));
    m.insert("de".to_string(), parse_locale(DE_JSON));
    m.insert("fr".to_string(), parse_locale(FR_JSON));
    m.insert("zh-cn".to_string(), parse_locale(ZH_CN_JSON));
    m
});

static CURRENT: Lazy<RwLock<String>> = Lazy::new(|| RwLock::new(DEFAULT_LANGUAGE.to_string()));

fn parse_locale(raw: &str) -> HashMap<String, String> {
    serde_json::from_str(raw).unwrap_or_else(|e| {
        panic!("invalid locale JSON: {e}");
    })
}

pub async fn init(pool: &SqlitePool) -> anyhow::Result<()> {
    let lang = crate::db::get_client_setting(pool, SETTING_LANGUAGE)
        .await?
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_LANGUAGE.to_string());
    set_locale(&lang);
    Ok(())
}

pub async fn set_language(pool: &SqlitePool, lang: &str) -> anyhow::Result<()> {
    let lang = normalize_lang(lang);
    crate::db::set_client_setting(pool, SETTING_LANGUAGE, &lang).await?;
    set_locale(&lang);
    Ok(())
}

pub fn current_language() -> String {
    CURRENT.read().unwrap().clone()
}

pub fn set_locale(lang: &str) {
    let lang = normalize_lang(lang);
    *CURRENT.write().unwrap() = lang;
}

fn normalize_lang(lang: &str) -> String {
    match lang.trim().to_lowercase().as_str() {
        "ru" | "ru-ru" => "ru".to_string(),
        "de" | "de-de" => "de".to_string(),
        "fr" | "fr-fr" => "fr".to_string(),
        "zh" | "zh-cn" | "zh-hans" | "zh_cn" => "zh-cn".to_string(),
        _ => "en".to_string(),
    }
}

fn lookup(lang: &str, key: &str) -> Option<String> {
    CATALOG.get(lang)?.get(key).cloned()
}

/// Translate key; fallback en → key itself.
pub fn t(key: &str) -> String {
    let lang = current_language();
    lookup(&lang, key)
        .or_else(|| lookup(DEFAULT_LANGUAGE, key))
        .unwrap_or_else(|| key.to_string())
}

/// Replace `{name}` placeholders in translated string.
pub fn t_fmt(key: &str, replacements: &[(&str, &str)]) -> String {
    let mut s = t(key);
    for (name, value) in replacements {
        s = s.replace(&format!("{{{name}}}"), value);
    }
    s
}

pub fn state_on_off(enabled: bool) -> String {
    if enabled {
        t("state.on")
    } else {
        t("state.off")
    }
}

pub fn encrypt_state(on: bool, na: bool) -> String {
    if na {
        t("encrypt.na")
    } else if on {
        t("encrypt.on")
    } else {
        t("encrypt.off")
    }
}

pub fn language_display_name(lang_code: &str) -> String {
    let key = match normalize_lang(lang_code).as_str() {
        "ru" => "lang.ru",
        "de" => "lang.de",
        "fr" => "lang.fr",
        "zh-cn" => "lang.zh_cn",
        _ => "lang.en",
    };
    t(key)
}
