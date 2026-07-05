//! Rustls server config from tls/ directory.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::ServerConfig;
use rustls_pemfile::{certs, pkcs8_private_keys};
use tracing::info;
use x509_parser::prelude::*;

use crate::config::Config;
use crate::i18n;
use crate::paths;

pub struct TlsInfo {
    pub subject_cn: String,
    pub not_after: DateTime<Utc>,
    pub days_remaining: i64,
}

pub fn cert_paths(config: &Config) -> Result<(std::path::PathBuf, std::path::PathBuf)> {
    let cert = paths::resolve(&config.tls_cert)?;
    let key = paths::resolve(&config.tls_key)?;
    Ok((cert, key))
}

pub fn files_present(config: &Config) -> Result<bool> {
    let (cert_path, key_path) = cert_paths(config)?;
    Ok(cert_path.is_file() && key_path.is_file())
}

pub fn files_present_at_default_paths() -> Result<bool> {
    let cert = paths::resolve("tls/server.crt")?;
    let key = paths::resolve("tls/server.key")?;
    Ok(cert.is_file() && key.is_file())
}

pub fn missing_tls_message(config: &Config) -> String {
    let (cert_path, key_path) = match cert_paths(config) {
        Ok(p) => p,
        Err(_) => (
            std::path::PathBuf::from("tls/server.crt"),
            std::path::PathBuf::from("tls/server.key"),
        ),
    };
    format!(
        "{}\n{}\n{}\n{}\n{}",
        i18n::t("tls.missing_title"),
        i18n::t("tls.missing_body"),
        i18n::t_fmt("tls.missing_cert", &[("path", &paths::display_path(&cert_path))]),
        i18n::t_fmt("tls.missing_key", &[("path", &paths::display_path(&key_path))]),
        i18n::t("tls.missing_how"),
    )
}

pub fn inspect_certificate(cert_path: &Path) -> Result<TlsInfo> {
    let pem = fs::read(cert_path)
        .with_context(|| format!("Failed to read TLS certificate: {}", cert_path.display()))?;
    let mut reader = pem.as_slice();
    let parsed: Vec<_> = certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to parse TLS certificate PEM")?;
    let der = parsed
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("TLS certificate file is empty"))?;

    let (_, cert) = X509Certificate::from_der(der.as_ref()).context("Invalid X.509 certificate")?;
    let not_after_ts = cert.validity().not_after.timestamp();
    let not_after_utc = DateTime::from_timestamp(not_after_ts, 0)
        .ok_or_else(|| anyhow::anyhow!("Invalid certificate not_after time"))?;
    let days_remaining = (not_after_utc - Utc::now()).num_days();

    let subject_cn = cert
        .subject()
        .iter_common_name()
        .next()
        .and_then(|cn| cn.as_str().ok())
        .unwrap_or("unknown")
        .to_string();

    Ok(TlsInfo {
        subject_cn,
        not_after: not_after_utc,
        days_remaining,
    })
}

pub fn validate_tls_startup(config: &Config) -> Result<TlsInfo> {
    let (cert_path, key_path) = cert_paths(config)?;

    if !cert_path.exists() || !key_path.exists() {
        bail!("{}", missing_tls_message(config));
    }

    let info = inspect_certificate(&cert_path)?;
    if info.days_remaining < 0 {
        bail!(
            "{}",
            i18n::t_fmt(
                "tls.expired",
                &[
                    ("date", &info.not_after.format("%Y-%m-%d %H:%M:%S UTC").to_string()),
                    ("cn", &info.subject_cn),
                ]
            )
        );
    }

    if info.days_remaining <= 30 {
        info!(
            "TLS certificate expires in {} day(s) on {} — renew soon",
            info.days_remaining,
            info.not_after.format("%Y-%m-%d")
        );
    } else {
        tracing::debug!(
            "TLS certificate valid for {} day(s) (CN: {}, expires {})",
            info.days_remaining,
            info.subject_cn,
            info.not_after.format("%Y-%m-%d")
        );
    }

    harden_private_key_permissions(&key_path);

    Ok(info)
}

#[allow(unused_variables)]
fn harden_private_key_permissions(key_path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        match fs::metadata(key_path) {
            Ok(meta) if meta.permissions().mode() & 0o077 != 0 => {
                if fs::set_permissions(key_path, fs::Permissions::from_mode(0o600)).is_ok() {
                    info!("Set TLS private key permissions to 600: {}", key_path.display());
                } else {
                    tracing::warn!(
                        "Could not chmod 600 {} (run as file owner or root)",
                        key_path.display()
                    );
                }
            }
            Ok(_) => {}
            Err(e) => tracing::debug!("Could not stat TLS key {}: {e}", key_path.display()),
        }
    }
}

pub fn load_rustls_config(config: &Config) -> Result<Arc<ServerConfig>> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let (cert_path, key_path) = cert_paths(config)?;

    let cert_pem = fs::read(&cert_path)
        .with_context(|| format!("Failed to read {}", cert_path.display()))?;
    let key_pem = fs::read(&key_path)
        .with_context(|| format!("Failed to read {}", key_path.display()))?;

    let mut cert_reader = cert_pem.as_slice();
    let cert_chain: Vec<CertificateDer<'static>> = certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to parse certificate chain")?
        .into_iter()
        .map(CertificateDer::from)
        .collect();

    if cert_chain.is_empty() {
        bail!("No certificates found in {}", cert_path.display());
    }

    let mut key_reader = key_pem.as_slice();
    let mut keys = pkcs8_private_keys(&mut key_reader)
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to parse private key")?;
    if keys.is_empty() {
        bail!("No private key found in {}", key_path.display());
    }

    let key = PrivateKeyDer::Pkcs8(keys.remove(0).into());

    let server_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key)
        .context("Invalid TLS certificate or key")?;

    Ok(Arc::new(server_config))
}
