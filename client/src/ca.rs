//! Local root CA and per-server TLS certificates (rcgen).

use std::fs;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use rcgen::{
    BasicConstraints, CertificateParams, DnType, IsCa, KeyPair,
};
use sha2::{Digest, Sha256};

use crate::paths;

const CA_VALIDITY_DAYS: u32 = 3650;
const SERVER_VALIDITY_DAYS: u32 = 730;

pub struct CaMaterial {
    pub root_cert_pem: String,
    pub root_key_pem: String,
}

pub struct ServerCertBundle {
    pub server_cert_pem: String,
    pub server_key_pem: String,
    pub fingerprint: String,
    pub not_after_days: u32,
}

pub fn ca_dir() -> PathBuf {
    paths::data_dir().join("ca")
}

pub fn root_cert_path() -> PathBuf {
    ca_dir().join("root.crt")
}

pub fn root_key_path() -> PathBuf {
    ca_dir().join("root.key")
}

pub fn agent_dir(slug: &str) -> PathBuf {
    ca_dir().join(slug)
}

pub fn ensure_ca() -> Result<CaMaterial> {
    fs::create_dir_all(ca_dir())?;

    let cert_path = root_cert_path();
    let key_path = root_key_path();

    if cert_path.exists() && key_path.exists() {
        return Ok(CaMaterial {
            root_cert_pem: fs::read_to_string(&cert_path)?,
            root_key_pem: fs::read_to_string(&key_path)?,
        });
    }

    let key_pair = KeyPair::generate()?;
    let mut params = CertificateParams::default();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params
        .distinguished_name
        .push(DnType::CommonName, "Backup Client Root CA");
    params
        .distinguished_name
        .push(DnType::OrganizationName, "Backup Client");
    params.not_before = time::OffsetDateTime::now_utc() - time::Duration::days(1);
    params.not_after =
        time::OffsetDateTime::now_utc() + time::Duration::days(i64::from(CA_VALIDITY_DAYS));

    let cert = params.self_signed(&key_pair)?;
    let root_cert_pem = cert.pem();
    let root_key_pem = key_pair.serialize_pem();

    fs::write(&cert_path, &root_cert_pem)?;
    fs::write(&key_path, &root_key_pem)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600))?;
    }

    Ok(CaMaterial {
        root_cert_pem,
        root_key_pem,
    })
}

pub fn recreate_root_ca() -> Result<CaMaterial> {
    let cert_path = root_cert_path();
    let key_path = root_key_path();
    if cert_path.exists() {
        fs::remove_file(&cert_path).context("Failed to remove old root.crt")?;
    }
    if key_path.exists() {
        fs::remove_file(&key_path).context("Failed to remove old root.key")?;
    }
    ensure_ca()
}

pub fn issue_server_cert(host: &str, slug: &str) -> Result<ServerCertBundle> {
    let ca = ensure_ca()?;
    host_endpoint(host)?;

    let ca_key = KeyPair::from_pem(&ca.root_key_pem)?;
    let ca_params = CertificateParams::from_ca_cert_pem(&ca.root_cert_pem)?;
    let ca_cert = ca_params.self_signed(&ca_key)?;

    let mut params = CertificateParams::new(vec![host.to_string()])?;
    params.is_ca = IsCa::NoCa;
    let server_key = KeyPair::generate()?;
    params
        .distinguished_name
        .push(DnType::CommonName, format!("backup-{slug}"));
    params.not_before = time::OffsetDateTime::now_utc() - time::Duration::days(1);
    params.not_after =
        time::OffsetDateTime::now_utc() + time::Duration::days(i64::from(SERVER_VALIDITY_DAYS));

    let server_cert = params.signed_by(&server_key, &ca_cert, &ca_key)?;
    let server_cert_pem = server_cert.pem();
    let server_key_pem = server_key.serialize_pem();
    let fingerprint = cert_fingerprint_pem(&server_cert_pem)?;

    Ok(ServerCertBundle {
        server_cert_pem,
        server_key_pem,
        fingerprint,
        not_after_days: SERVER_VALIDITY_DAYS,
    })
}

pub fn cert_fingerprint_pem(pem: &str) -> Result<String> {
    let der = pem_to_first_der(pem)?;
    Ok(cert_fingerprint_der(&der))
}

pub fn cert_fingerprint_der(der: &[u8]) -> String {
    hex::encode(Sha256::digest(der))
}

pub fn load_root_cert_pem() -> Result<String> {
    ensure_ca()?;
    fs::read_to_string(root_cert_path()).context("Failed to read root CA certificate")
}

fn pem_to_first_der(pem: &str) -> Result<Vec<u8>> {
    let mut reader = pem.as_bytes();
    let certs: Vec<_> = rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to parse PEM certificate")?;
    certs
        .into_iter()
        .next()
        .map(|c| c.to_vec())
        .ok_or_else(|| anyhow::anyhow!("PEM contains no certificate"))
}

fn host_endpoint(host: &str) -> Result<()> {
    let host = host.trim();
    if host.parse::<std::net::IpAddr>().is_ok() {
        return Ok(());
    }
    if host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && !host.is_empty()
    {
        return Ok(());
    }
    bail!("Invalid host for certificate: {host}");
}

pub fn parse_host_from_server_addr(addr: &str) -> Result<String> {
    let addr = addr.trim();
    let host = addr
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or(addr);
    Ok(host.to_string())
}
