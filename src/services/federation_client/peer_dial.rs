// EasyNet CLI — federation_client — peer_dial helpers
// =====================================================
//
// File: src/services/federation_client/peer_dial.rs
// Description: Pure-function TLS-pinning helpers shared by every
//              outbound dial that targets a hub-mode peer
//              (`cross_hub_dial::resolve_peer_channel` for hub-to-hub
//              cross-realm forward_invoke, `axon_serve::session_
//              initiator::dial_and_run_session` for device-to-hub
//              `<self>.session` bootstrap).
//
// Why this module exists
// ----------------------
// LB-39 §26 (C2a) and LB-48 share the same shape: read a per-peer
// CA PEM file from disk + build a `tonic::transport::ClientTlsConfig`
// pinned to that CA + apply it to a `tonic::transport::Endpoint`.
// Two call sites used to inline four near-identical lines each;
// extraction de-duplicates them and gives both call sites a single
// audited code path for SDK-conformance review.
//
// The helper is *pure*: it takes a `&Path`, returns a
// `Result<ClientTlsConfig, PinnedTlsError>`. Endpoint construction
// + `tls_config(...)` application stays at the call site so each
// caller can wrap the typed error into its own error variant
// (`SessionError::TlsCaRead` vs `FederationClientError::DialFailed`)
// without leaking a federation-client error type into the session
// initiator.
//
// Why not a bigger helper that also takes the Endpoint
// ----------------------------------------------------
// The two call sites' Endpoint construction looks identical at
// first glance — `Endpoint::from_shared(uri)` then optional
// `connect_timeout` — but they map errors differently:
//   - cross_hub_dial maps to `FederationClientError::DialFailed`
//   - session_initiator maps to `SessionError::InvalidEndpoint`
// Both wrap the underlying `tonic::transport::Error` with extra
// context (the hub URI string, the CA file path). Bundling the
// Endpoint construction into the helper would force a single shared
// error type and lose that context. Keeping the helper at the
// `ClientTlsConfig` boundary preserves caller-side error
// formatting while still removing the duplicate PEM-read +
// Certificate::from_pem + ClientTlsConfig::new chain.
//
// DEC-N1 reminder
// ---------------
// There is no system-CA fallback by design: a peer entry without a
// `tls_ca_pem_path` is `PeerNotTrusted`, not "fall back to system
// trust roots". Both call sites enforce that policy upstream of
// this helper. The helper itself only handles the `Some(path)`
// arm; `None` is the caller's branch to handle.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::{Path, PathBuf};

use tonic::transport::{Certificate, ClientTlsConfig};

/// Typed failure variants for [`pinned_tls_config`]. Callers wrap
/// these into their own error families (e.g.
/// `FederationClientError::DialFailed { hub, detail }` or
/// `SessionError::TlsCaRead { path, source }`) so each call site's
/// upstream error contract stays intact.
#[derive(Debug)]
pub enum PinnedTlsError {
    /// `std::fs::read` of the PEM file failed. Most often a
    /// missing file (operator typo in `realm-trust.toml`'s
    /// `tls_ca_pem_path`) or a permissions issue.
    ReadFailed {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl std::fmt::Display for PinnedTlsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PinnedTlsError::ReadFailed { path, source } => {
                write!(f, "read tls_ca_pem_path `{}`: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for PinnedTlsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PinnedTlsError::ReadFailed { source, .. } => Some(source),
        }
    }
}

/// Build a `ClientTlsConfig` pinned to the operator-supplied CA
/// at `ca_pem_path`. The PEM is read every call — channel pools
/// already cache the resulting `Channel`, so re-reading the PEM on
/// a cache miss is the right granularity (a SIGHUP-driven cert
/// rotation that bumps `cert_anchor_generation` invalidates the
/// cached channel and forces a fresh read).
///
/// `tonic::transport::Certificate::from_pem` is infallible — it
/// stores the bytes verbatim and defers parse errors to handshake
/// time, which then surface as `tonic::transport::Error` from the
/// caller's `endpoint.tls_config(...)` or `endpoint.connect()`. So
/// the only typed error this helper emits is `ReadFailed`.
pub fn pinned_tls_config(ca_pem_path: &Path) -> Result<ClientTlsConfig, PinnedTlsError> {
    let ca_pem = std::fs::read(ca_pem_path).map_err(|err| PinnedTlsError::ReadFailed {
        path: ca_pem_path.to_path_buf(),
        source: err,
    })?;
    let ca = Certificate::from_pem(&ca_pem);
    Ok(ClientTlsConfig::new().ca_certificate(ca))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_tls_config_reads_real_pem() {
        // A minimal valid PEM is enough — `Certificate::from_pem`
        // is infallible (parse errors defer to handshake time).
        // The helper's contract is "read PEM bytes from disk + wrap
        // them into ClientTlsConfig"; a successful return + non-
        // surprising PathBuf metadata is the assertion.
        let dir = tempfile::tempdir().expect("tempdir");
        let pem = dir.path().join("ca.pem");
        std::fs::write(
            &pem,
            b"-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n",
        )
        .expect("seed pem");
        let _config = pinned_tls_config(&pem).expect("pin should succeed");
    }

    #[test]
    fn pinned_tls_config_surfaces_typed_error_on_missing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("does-not-exist.pem");
        let err = pinned_tls_config(&missing).expect_err("missing file must fail");
        match err {
            PinnedTlsError::ReadFailed { path, source } => {
                assert_eq!(path, missing);
                assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
            }
        }
    }

    #[test]
    fn pinned_tls_error_display_includes_path_and_source() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("absent.pem");
        let err = pinned_tls_config(&missing).expect_err("missing file");
        let rendered = err.to_string();
        assert!(rendered.contains(&missing.display().to_string()));
        // The OS-formatted source ("No such file or directory" /
        // localised equivalent) appears too — we don't pin the
        // exact substring since it is localised.
        assert!(rendered.contains("read tls_ca_pem_path"));
    }
}
