//! Connection record (slug, WSS URL, API key, TLS).

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::i18n;
use crate::ui;

#[derive(Clone, Debug, Serialize, Deserialize, FromRow)]
pub struct Connection {
    pub id: String,
    pub slug: String,
    pub url: String,
    pub api_key: String,
    pub tls_enabled: bool,
    pub cert_fingerprint: Option<String>,
    pub enabled: bool,
    pub created_at: String,
    #[sqlx(default)]
    pub secrets_mode: String,
    #[sqlx(default)]
    pub retention_days: i32,
    pub retention_last_run: Option<String>,
}

impl Connection {
    pub fn label(&self) -> String {
        format!("{} | {}", ui::host_from_url(&self.url), self.slug)
    }

    pub fn is_production(&self) -> bool {
        self.secrets_mode == "production"
    }

    pub fn secrets_mode_label(&self) -> String {
        if self.is_production() {
            i18n::t("secrets.production")
        } else {
            i18n::t("secrets.test")
        }
    }

    pub fn retention_short(&self) -> String {
        match self.retention_days {
            0 => i18n::t("retention.short_never"),
            7 => i18n::t("retention.short_7d"),
            14 => i18n::t("retention.short_14d"),
            30 => i18n::t("retention.short_30d"),
            60 => i18n::t("retention.short_60d"),
            _ => i18n::t("retention.short_unknown"),
        }
    }
}
