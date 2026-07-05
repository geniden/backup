//! Console messages: embedded en/ru/de/fr/zh-CN JSON. Locale from BACKUP_LANG or LANG.

use std::collections::HashMap;
use std::sync::OnceLock;

const EN_JSON: &str = include_str!("en.json");
const RU_JSON: &str = include_str!("ru.json");
const DE_JSON: &str = include_str!("de.json");
const FR_JSON: &str = include_str!("fr.json");
const ZH_CN_JSON: &str = include_str!("zh-CN.json");

pub const DEFAULT_LANGUAGE: &str = "en";

static CATALOG: OnceLock<HashMap<String, HashMap<String, String>>> = OnceLock::new();
static CURRENT: OnceLock<std::sync::RwLock<String>> = OnceLock::new();

fn catalog() -> &'static HashMap<String, HashMap<String, String>> {
    CATALOG.get_or_init(|| {
        let mut m = HashMap::new();
        m.insert("en".to_string(), parse_locale(EN_JSON));
        m.insert("ru".to_string(), parse_locale(RU_JSON));
        m.insert("de".to_string(), parse_locale(DE_JSON));
        m.insert("fr".to_string(), parse_locale(FR_JSON));
        m.insert("zh-cn".to_string(), parse_locale(ZH_CN_JSON));
        m
    })
}

fn current_lock() -> &'static std::sync::RwLock<String> {
    CURRENT.get_or_init(|| std::sync::RwLock::new(DEFAULT_LANGUAGE.to_string()))
}

fn parse_locale(raw: &str) -> HashMap<String, String> {
    serde_json::from_str(raw).unwrap_or_else(|e| {
        panic!("invalid server locale JSON: {e}");
    })
}

pub fn init() {
    let lang = detect_lang();
    set_locale(&lang);
}

fn detect_lang() -> String {
    if let Ok(v) = std::env::var("BACKUP_LANG") {
        return normalize_lang(&v);
    }
    if let Ok(v) = std::env::var("LANG") {
        return normalize_lang(&v);
    }
    DEFAULT_LANGUAGE.to_string()
}

fn normalize_lang(lang: &str) -> String {
    let lang = lang.trim().to_lowercase();
    if lang.starts_with("ru") {
        "ru".to_string()
    } else if lang.starts_with("de") {
        "de".to_string()
    } else if lang.starts_with("fr") {
        "fr".to_string()
    } else if lang.starts_with("zh") {
        "zh-cn".to_string()
    } else {
        "en".to_string()
    }
}

pub fn set_locale(lang: &str) {
    *current_lock().write().unwrap() = normalize_lang(lang);
}

fn lookup(lang: &str, key: &str) -> Option<String> {
    catalog().get(lang)?.get(key).cloned()
}

pub fn t(key: &str) -> String {
    let lang = current_lock().read().unwrap().clone();
    lookup(&lang, key)
        .or_else(|| lookup(DEFAULT_LANGUAGE, key))
        .unwrap_or_else(|| key.to_string())
}

pub fn t_fmt(key: &str, replacements: &[(&str, &str)]) -> String {
    let mut s = t(key);
    for (name, value) in replacements {
        s = s.replace(&format!("{{{name}}}"), value);
    }
    s
}

pub fn print_key(key: &str) {
    print!("{}", t(key));
}
