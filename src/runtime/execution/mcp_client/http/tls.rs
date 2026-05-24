// EasyNet CLI — Streamable HTTP / TLS connector
// =============================================
//
// File: src/runtime/execution/mcp_client/http/tls.rs
//
// Builds a `tokio_rustls::TlsConnector` for one MCP server's
// per-spec TLS posture:
//
//   * Mozilla roots by default (`webpki-roots`).
//   * Optional private CA bundle appended at config-load time.
//   * Double-gated `insecure_skip_verify` — refused unless the
//     daemon was started with `EASYNET_ALLOW_INSECURE_TLS=1`. An
//     attacker who can only write the config file cannot silently
//     downgrade TLS verification.
//
// Also exposes the [`AsyncStream`] marker trait so the same
// connection-driving code drives both plain `TcpStream` and
// `tokio_rustls::client::TlsStream<TcpStream>` through one
// `Box<dyn AsyncStream>`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use anyhow::Context;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::runtime::execution::mcp_client::TlsSpec;

/// Marker trait so `HyperTokioIo` can wrap either a plain
/// `TcpStream` or a `tokio_rustls::client::TlsStream<TcpStream>`.
/// Both already satisfy `AsyncRead + AsyncWrite + Unpin + Send`,
/// so this is a pure type alias with no behaviour.
pub(super) trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncStream for T {}

/// Build a rustls TLS connector for one MCP server. Mozilla roots
/// from `webpki-roots` are the default trust source; an operator
/// can append a private CA via `TlsSpec.ca_bundle` (PEM file). The
/// double-gated `insecure_skip_verify` path is rejected unless the
/// daemon was started with `EASYNET_ALLOW_INSECURE_TLS=1`, so an
/// attacker who can only write the config file cannot silently
/// downgrade TLS verification.
pub(super) fn build_tls_connector(
    spec: &TlsSpec,
    server_label: &str,
) -> anyhow::Result<tokio_rustls::TlsConnector> {
    use rustls::pki_types::CertificateDer;
    use rustls::{ClientConfig, RootCertStore};

    let mut roots = RootCertStore::empty();
    // webpki-roots ships Mozilla's CA list as a const slice of
    // `TrustAnchor`s. Calling `extend` here adds every public CA
    // that browsers trust by default.
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    if let Some(ca_path) = spec.ca_bundle.as_deref() {
        let raw = std::fs::read(ca_path)
            .with_context(|| format!("read CA bundle `{}`", ca_path.display()))?;
        let mut cursor = std::io::Cursor::new(raw);
        let mut added = 0usize;
        for cert in rustls_pemfile::certs(&mut cursor) {
            let cert: CertificateDer<'_> = cert.with_context(|| {
                format!("parse certificate in CA bundle `{}`", ca_path.display())
            })?;
            roots
                .add(cert)
                .with_context(|| format!("trust CA from `{}`", ca_path.display()))?;
            added += 1;
        }
        if added == 0 {
            anyhow::bail!(
                "MCP server `{server_label}`: tls.ca_bundle `{}` contained no \
                 valid CERTIFICATE PEM blocks",
                ca_path.display()
            );
        }
    }

    let config = if spec.insecure_skip_verify {
        if std::env::var("EASYNET_ALLOW_INSECURE_TLS").ok().as_deref() != Some("1") {
            anyhow::bail!(
                "MCP server `{server_label}`: tls.insecure_skip_verify requested \
                 but daemon was not started with EASYNET_ALLOW_INSECURE_TLS=1. \
                 Refusing to disable certificate verification."
            );
        }
        // Security audit trail. SRE pipelines can grep
        // `kind=tls_insecure` to alert on any host where
        // insecure_skip_verify has been enabled.
        crate::op_event!(
            component = mcp_http_client,
            kind = tls_insecure,
            server = server_label,
            level = "warn",
            message = "TLS certificate verification disabled via tls.insecure_skip_verify=true; \
                      DO NOT use outside closed test environments",
        );
        ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(InsecureCertVerifier))
            .with_no_client_auth()
    } else {
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth()
    };

    Ok(tokio_rustls::TlsConnector::from(Arc::new(config)))
}

/// **DANGER**: accepts any server certificate, of any name, signed
/// by anyone (including expired or self-signed). Only used when
/// `TlsSpec.insecure_skip_verify` is true AND the daemon was
/// started with `EASYNET_ALLOW_INSECURE_TLS=1`.
#[derive(Debug)]
struct InsecureCertVerifier;

impl rustls::client::danger::ServerCertVerifier for InsecureCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        // Accept every scheme rustls supports. We've already
        // promised not to verify anything, so the set just has to
        // cover whatever a server might pick.
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insecure_skip_verify_refused_without_env_gate() {
        // Env var is not "1" — the operator config alone must NOT
        // be enough to disable verification. This is the second
        // half of the double gate.
        std::env::remove_var("EASYNET_ALLOW_INSECURE_TLS");
        let spec = TlsSpec {
            insecure_skip_verify: true,
            ..TlsSpec::default()
        };
        let err = match build_tls_connector(&spec, "test-server") {
            Ok(_) => panic!("expected double-gate refusal, got Ok"),
            Err(e) => e,
        };
        let msg = format!("{err}");
        assert!(msg.contains("EASYNET_ALLOW_INSECURE_TLS=1"));
        assert!(msg.contains("Refusing to disable"));
    }

    #[test]
    fn default_spec_builds_with_public_roots() {
        let spec = TlsSpec::default();
        assert!(
            build_tls_connector(&spec, "test-server").is_ok(),
            "default TlsSpec must build a connector against Mozilla roots"
        );
    }
}
