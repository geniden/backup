//! WSS/HTTPS with certificate fingerprint pinning (TLS always required).

use std::sync::Arc;

use anyhow::{Context, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, Error, RootCertStore, SignatureScheme};
use rustls_pemfile::certs;
use tokio_tungstenite::connect_async_tls_with_config;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream};
use tokio::net::TcpStream;

use crate::ca;
use crate::models::connection::Connection;

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Debug)]
struct PinnedVerifier {
    roots: Arc<RootCertStore>,
    expected_fingerprint: String,
}

impl PinnedVerifier {
    fn new(root_pem: &str, expected_fingerprint: &str) -> Result<Self> {
        let mut roots = RootCertStore::empty();
        let mut reader = root_pem.as_bytes();
        let parsed: Vec<_> = certs(&mut reader)
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to parse root CA PEM")?;
        for cert in parsed {
            roots
                .add(cert)
                .context("Failed to add root CA to trust store")?;
        }
        Ok(Self {
            roots: Arc::new(roots),
            expected_fingerprint: expected_fingerprint.to_ascii_lowercase(),
        })
    }
}

impl ServerCertVerifier for PinnedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let actual = ca::cert_fingerprint_der(end_entity.as_ref());
        if actual != self.expected_fingerprint {
            return Err(Error::General(format!(
                "Certificate fingerprint mismatch (expected {}, got {})",
                self.expected_fingerprint, actual
            )));
        }

        rustls::client::WebPkiServerVerifier::builder(self.roots.clone())
            .build()
            .map_err(|e| Error::General(e.to_string()))?
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::client::WebPkiServerVerifier::builder(self.roots.clone())
            .build()
            .map_err(|e| Error::General(e.to_string()))?
            .verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::client::WebPkiServerVerifier::builder(self.roots.clone())
            .build()
            .map_err(|e| Error::General(e.to_string()))?
            .verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

pub fn build_tls_config(conn: &Connection) -> Result<Arc<ClientConfig>> {
    ensure_crypto_provider();

    let fingerprint = conn
        .cert_fingerprint
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("TLS enabled but cert_fingerprint is missing"))?;
    let root_pem = ca::load_root_cert_pem()?;
    let verifier = Arc::new(PinnedVerifier::new(&root_pem, fingerprint)?);

    let config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();

    Ok(Arc::new(config))
}

fn ensure_crypto_provider() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        rustls::crypto::ring::default_provider()
            .install_default()
            .expect("install rustls ring crypto provider");
    });
}

fn tls_config_for_http(conn: &Connection) -> Result<ClientConfig> {
    let arc = build_tls_config(conn)?;
    let mut config = match Arc::try_unwrap(arc) {
        Ok(c) => c,
        Err(shared) => (*shared).clone(),
    };
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(config)
}

pub async fn connect_ws(conn: &Connection) -> Result<(WsStream, tokio_tungstenite::tungstenite::http::Response<Option<Vec<u8>>>)> {
    let tls_config = build_tls_config(conn)?;
    let connector = Connector::Rustls(tls_config);
    connect_async_tls_with_config(
        &conn.url,
        Some(WebSocketConfig::default()),
        false,
        Some(connector),
    )
    .await
    .map_err(|e| map_tls_error(e, conn))
}

pub fn build_http_client(conn: &Connection) -> Result<reqwest::Client> {
    let tls_config = tls_config_for_http(conn)?;
    reqwest::Client::builder()
        .use_preconfigured_tls(tls_config)
        .build()
        .with_context(|| format!("[{}] Failed to build HTTPS client", conn.slug))
}

pub fn normalize_download_url(_conn: &Connection, url: &str) -> String {
    url.replace("http://", "https://")
}

fn map_tls_error(
    e: tokio_tungstenite::tungstenite::Error,
    conn: &Connection,
) -> anyhow::Error {
    let msg = e.to_string();
    if msg.contains("CertificateExpired")
        || msg.contains("expired")
        || msg.contains("NotValidYet")
    {
        anyhow::anyhow!(
            "[{}] Server TLS certificate expired or not yet valid — renew from the client menu",
            conn.slug
        )
    } else if msg.contains("fingerprint mismatch") {
        anyhow::anyhow!(
            "[{}] TLS certificate fingerprint mismatch — renew certificate (data/ca/{})",
            conn.slug,
            conn.slug
        )
    } else {
        anyhow::anyhow!("[{}] TLS/WebSocket error: {msg}", conn.slug)
    }
}
