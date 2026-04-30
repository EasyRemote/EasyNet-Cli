// EasyNet CLI — Federation Invoke Helper (PR-N1 commit 8/N)
// =========================================================
//
// File: src/support/federation_invoke.rs
// Description: CLI bridge from `easynet ability invoke <ability>
//              --node <peer-uri>` to the local daemon's gRPC
//              `Invocation::invoke` with `function_name="federation.
//              forward_invoke"`. This is what unblocks the answer-
//              sheet acceptance gate 晓雯 letter 61 + 海峰 letter 62 +
//              凉冰 LB-35 raised: PR-N1 commits 1-7/N ship the
//              daemon-to-daemon wire, this commit ships the user-
//              CLI-command → daemon entry point.
//
// Wire shape
// ----------
// 1. Caller passes `(ability, args, node_uri)`. `node_uri` must be
//    a canonical `easynet:///r/{tenant}/agent/{node}` URI; non-
//    canonical inputs surface as a typed error before any IPC.
// 2. The helper dials the local daemon's UDS-bound gRPC
//    `Invocation` server (default `~/.easynet/daemon.sock`).
// 3. It sends an `InvokeRequest` whose `function_name` is
//    `federation.forward_invoke` and whose `arguments` is the
//    JSON `{target_uri, inner_envelope_b64}` shape that
//    `federation_wrappers::ForwardInvokeRequest` deserialises.
//    The inner envelope carries the original `(ability, args)`
//    pair so the peer-side daemon can dispatch it once it lands.
// 4. The local daemon's `dispatch_federation_forward_invoke` runs
//    the cross-tenant routing landed in PR-N1 commit 3b/N, dials
//    via the `CrossHubDialer` that PR-N1 commit 6/N wires at boot,
//    and returns the peer daemon's response.
//
// Why this lives in `support/` and not `facade/cli/`
// ---------------------------------------------------
// The IPC + gRPC plumbing this helper does is reusable across CLI
// subcommands (a future `easynet mission run --node X` would do
// the same dial + frame). Putting it under `support/` matches the
// existing `local_invoke.rs` placement (the helper that backs every
// CLI subcommand's local-only path) so a reader looking for
// "where does the CLI talk to the daemon" finds both helpers in
// one directory.
//
// Feature gating
// --------------
// `axon-pb` feature gates the entire module. Production builds run
// with `axon-pb` on; minimal builds (no daemon transport) can drop
// the cross-hub bridge along with the rest of the gRPC stack.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

#![cfg(feature = "axon-pb")]

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use serde_json::{Value, json};
use tonic::transport::{Endpoint, Uri};
use tower::service_fn;

use crate::pb::axon::v1::invocation_client::InvocationClient;
use crate::pb::axon::v1::{AgentIdentity, Envelope, InvokeRequest};

/// Default UDS path the daemon binds for its `Invocation` gRPC
/// server. Mirrors `persistence::daemon_config::DEFAULT_DAEMON_UDS_PATH`
/// since the same tilde-expanded path is the wire contract.
const DEFAULT_DAEMON_GRPC_UDS_PATH: &str = "~/.easynet/daemon.sock";

/// Validate a `--node` argument as a canonical EasyNet agent URI.
/// Returns the URI string when it parses; surfaces a typed error
/// when it doesn't, with the exact wire-shape we expect quoted in
/// the message so the operator can fix the typo.
pub fn parse_node_uri(node: &str) -> anyhow::Result<String> {
    let trimmed = node.trim();
    if !trimmed.starts_with("easynet:///r/") {
        bail!(
            "--node `{trimmed}` is not a canonical EasyNet agent URI. \
             Expected shape: easynet:///r/<tenant>/agent/<node>. \
             A bare hostname or `https://...` URL is not accepted — \
             pass the URI you got from `easynet discover` or your \
             pairing flow."
        );
    }
    // Past the `r/` prefix, require at least `<tenant>/agent/<node>`
    // so the daemon's `parse_tenant_from_uri` (PR-N1 commit 3a/N)
    // has a non-empty tenant component to extract.
    let after = trimmed
        .strip_prefix("easynet:///r/")
        .expect("prefix checked above");
    let mut parts = after.splitn(3, '/');
    let tenant = parts.next().unwrap_or("");
    let agent_keyword = parts.next().unwrap_or("");
    let node_id = parts.next().unwrap_or("");
    if tenant.is_empty() || agent_keyword != "agent" || node_id.is_empty() {
        bail!(
            "--node `{trimmed}` does not parse as easynet:///r/<tenant>/agent/<node>. \
             Got tenant={tenant:?}, segment={agent_keyword:?}, node={node_id:?}. \
             Pass the canonical URI from `easynet discover`."
        );
    }
    Ok(trimmed.to_string())
}

/// Dispatch `(ability, args)` against `node_uri` via the local
/// daemon's `federation.forward_invoke` ability. Synchronous
/// wrapper around the async tonic call so the caller (the
/// `easynet ability invoke` subcommand) keeps its sync shape.
///
/// Returns the inner ability's response value as JSON. Errors:
/// - daemon down (UDS connect refused) → `daemon not running`
/// - daemon admission rejected → `daemon error: <code>: <message>`
/// - peer offline / cross-hub dial failed → JSON
///   `{ target_online: false, ... }` (per PR-N1 spec) wrapped as
///   `Ok(Value)` so the caller can surface the structured outcome
pub fn invoke_via_federation_forward(
    ability: &str,
    args: Value,
    node_uri: &str,
    caller_uri: Option<&str>,
) -> anyhow::Result<Value> {
    let socket_path = expand_home(DEFAULT_DAEMON_GRPC_UDS_PATH);
    if !socket_path.exists() {
        bail!(
            "daemon not running (no gRPC socket at {}). \
             Start it with `easynet runtime start`.",
            socket_path.display()
        );
    }

    // The daemon's `federation.forward_invoke` request shape is
    // `ForwardInvokeRequest { target_uri, inner_envelope_b64 }`.
    // The inner envelope carries the original `(ability, args)`
    // tuple as a JSON blob; PR-N1's daemon-to-daemon wire
    // forwards this opaquely. PR-N2 will introduce AXIOM
    // mapping rewrite + signature; PR-N1 ships the unsigned shape.
    let inner_payload = json!({
        "ability": ability,
        "args": args,
    });
    let inner_envelope_bytes = serde_json::to_vec(&inner_payload)
        .context("serialise inner ability call as forward_invoke payload")?;
    let inner_envelope_b64 = base64_engine_encode(&inner_envelope_bytes);

    let forward_args = json!({
        "target_uri": node_uri,
        "inner_envelope_b64": inner_envelope_b64,
    });
    let forward_args_bytes = serde_json::to_vec(&forward_args)
        .context("serialise ForwardInvokeRequest")?;

    // Build the outer envelope. The caller URI defaults to a
    // generic `easynet:///r/cli/agent/local` so the daemon's
    // admission gate has something to log; production deployments
    // will override this with the operator's identity once
    // PR-N2 cross-realm signing lands.
    let resolved_caller_uri = caller_uri
        .unwrap_or("easynet:///r/cli/agent/local")
        .to_string();
    let envelope = Envelope {
        caller: Some(AgentIdentity {
            uri: resolved_caller_uri,
            ..AgentIdentity::default()
        }),
        ..Envelope::default()
    };

    let request = InvokeRequest {
        envelope: Some(envelope),
        function_name: "federation.forward_invoke".to_string(),
        arguments: forward_args_bytes,
        ..InvokeRequest::default()
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime for federation invoke")?;

    runtime.block_on(async move {
        let socket = socket_path.clone();
        let endpoint = Endpoint::try_from("http://[::1]:50051") // dummy URI for tonic;
            // the connector below replaces the network with UDS
            .context("build tonic endpoint")?
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10));
        let channel = endpoint
            .connect_with_connector(service_fn(move |_: Uri| {
                let path = socket.clone();
                async move {
                    let stream = tokio::net::UnixStream::connect(path)
                        .await?;
                    Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
                }
            }))
            .await
            .context("connect to local daemon gRPC UDS socket")?;

        let mut client = InvocationClient::new(channel);
        let response = client
            .invoke(request)
            .await
            .map_err(|status| {
                anyhow!(
                    "daemon error invoking federation.forward_invoke for target `{node_uri}` \
                     (code={:?}): {}",
                    status.code(),
                    status.message(),
                )
            })?;
        let body = response.into_inner();
        // The daemon's ForwardInvokeResponse shape is
        // `{target_online: bool}` for v1 — PR-N1 commits 3b/N +
        // 6/N forward the response verbatim. Parse + return as
        // JSON so the caller can decide how to present it.
        let parsed: Value = serde_json::from_slice(&body.result).with_context(|| {
            format!(
                "parse federation.forward_invoke response: result_content_type={:?}",
                body.result_content_type
            )
        })?;
        Ok(parsed)
    })
}

/// Tilde-expand `~/...` paths the same way the rest of the daemon
/// codebase does. Centralised here so the helper does not depend
/// on `services::axon_serve::boot::expand_home` (which lives behind
/// a feature wall).
fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

/// Minimal base64 encoder used for the inner envelope payload.
/// Avoids pulling `base64` as a top-level dep just for one
/// call site; the inner envelope is a CLI-internal blob, not a
/// wire-stable string, so a per-call encoder is fine. Matches
/// the standard alphabet so a daemon-side decoder using the
/// `base64` crate would interoperate.
fn base64_engine_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let combined = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((combined >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((combined >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((combined >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(combined & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_node_uri_accepts_canonical_shape() {
        let parsed =
            parse_node_uri("easynet:///r/realm-b/agent/laptop-bob").expect("canonical accepted");
        assert_eq!(parsed, "easynet:///r/realm-b/agent/laptop-bob");
    }

    #[test]
    fn parse_node_uri_trims_surrounding_whitespace() {
        let parsed = parse_node_uri("  easynet:///r/realm-b/agent/n1  \n")
            .expect("whitespace trimmed");
        assert_eq!(parsed, "easynet:///r/realm-b/agent/n1");
    }

    #[test]
    fn parse_node_uri_rejects_https_url() {
        let err = parse_node_uri("https://hub.example/r/foo").expect_err("https rejected");
        assert!(
            format!("{err}").contains("not a canonical EasyNet agent URI"),
            "error message must cite the canonical-URI requirement"
        );
    }

    #[test]
    fn parse_node_uri_rejects_bare_hostname() {
        let err = parse_node_uri("hub.example.com").expect_err("bare hostname rejected");
        assert!(format!("{err}").contains("canonical"));
    }

    #[test]
    fn parse_node_uri_rejects_missing_tenant() {
        let err = parse_node_uri("easynet:///r//agent/n1").expect_err("empty tenant rejected");
        assert!(
            format!("{err}").contains("does not parse"),
            "must surface a structural-mismatch error, got: {err}"
        );
    }

    #[test]
    fn parse_node_uri_rejects_missing_node_id() {
        let err = parse_node_uri("easynet:///r/realm-b/agent/")
            .expect_err("empty node id rejected");
        assert!(format!("{err}").contains("does not parse"));
    }

    #[test]
    fn parse_node_uri_rejects_wrong_keyword() {
        // The path component after the tenant MUST be `agent`. A
        // typo like `device` is rejected so the operator notices
        // before the URI hits the daemon.
        let err = parse_node_uri("easynet:///r/realm-b/device/n1")
            .expect_err("wrong keyword rejected");
        assert!(format!("{err}").contains("does not parse"));
    }

    #[test]
    fn base64_encode_round_trip_against_known_vectors() {
        // RFC 4648 §10 test vectors. Pin the encoder so a regression
        // here doesn't silently break the inner envelope shape.
        assert_eq!(base64_engine_encode(b""), "");
        assert_eq!(base64_engine_encode(b"f"), "Zg==");
        assert_eq!(base64_engine_encode(b"fo"), "Zm8=");
        assert_eq!(base64_engine_encode(b"foo"), "Zm9v");
        assert_eq!(base64_engine_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_engine_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_engine_encode(b"foobar"), "Zm9vYmFy");
    }
}
