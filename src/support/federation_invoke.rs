// EasyNet CLI — Federation Invoke Helper (PR-N1 commit 8/N)
// =========================================================
//
// File: src/support/federation_invoke.rs
// Description: CLI bridge from `easynet ability invoke <ability>
//              --node <peer-uri>` to the local daemon's gRPC
//              `Invocation::invoke` with `function_name="federation.
//              forward_invoke"`. This is what unblocks the answer-
//              sheet acceptance gate raised by perf-engineer +
//              reviewer + architect during PR-N1 review:
//              commits 1-7/N ship the
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

use anyhow::{anyhow, bail, Context};
use serde_json::{json, Value};
use tonic::transport::{Endpoint, Uri};
use tower::service_fn;

use crate::pb::axon::v1::invocation_client::InvocationClient;
use crate::pb::axon::v1::{AgentIdentity, Envelope, InvokeRequest, SubjectIdentity};

/// Default UDS path the daemon binds for its `Invocation` gRPC
/// server. Mirrors `persistence::daemon_config::DEFAULT_DAEMON_UDS_PATH`
/// since the same tilde-expanded path is the wire contract.
const DEFAULT_DAEMON_GRPC_UDS_PATH: &str = "~/.easynet/daemon.sock";

/// Validate a `--node` argument as a canonical EasyNet device URI.
/// Returns the URI string when it parses; surfaces a typed error
/// when it doesn't, with the exact wire-shape we expect quoted in
/// the message so the operator can fix the typo.
///
/// URI v4.1.4 (Phase 2F): `--node` carries a `device` URA, not an
/// `agent` URA. Devices are physical hosts (the box running the
/// daemon); agents are user-owned hosted profiles that live ON
/// devices. The legacy `/agent/<node>` shape collapsed both into
/// one segment; v4.1.4 splits them. We accept that legacy device-
/// as-agent form during the migration window and canonicalise it
/// into `/device/<node>` before the request hits presence lookup.
/// Real agent URIs (`/agent/<user>.<agent>`) are rejected: cross-
/// hub `federation.forward_invoke` targets devices, not hosted
/// user agents.
pub fn parse_node_uri(node: &str) -> anyhow::Result<String> {
    let trimmed = node.trim();
    if !trimmed.starts_with("easynet:///r/") {
        bail!(
            "--node `{trimmed}` is not a canonical EasyNet device URI. \
             Expected shape: easynet:///r/<realm>/device/<node-uuid>. \
             A bare hostname or `https://...` URL is not accepted. \
             {URI_DISCOVERY_HINT}"
        );
    }
    // Past the `r/` prefix, require at least `<realm>/<role>/<node>`
    // so the daemon's parsers have non-empty tenant + node
    // components to extract. Accept `device` directly; accept
    // legacy `agent/<node>` only when the tail is the old bare
    // node-id form, then rewrite it to the canonical device URI.
    let after = trimmed
        .strip_prefix("easynet:///r/")
        .expect("prefix checked above");
    let mut parts = after.splitn(3, '/');
    let realm = parts.next().unwrap_or("");
    let role = parts.next().unwrap_or("");
    let node_id = parts.next().unwrap_or("");

    // URI v4.1.4: `/hub` is a complete URA on its own (no id-tail);
    // it identifies the realm's singleton hub. `--node easynet:///r/<realm>/hub`
    // is a legitimate target for cross-hub federation.heartbeat /
    // federation.* calls — the daemon's local presence either
    // matches it (when the daemon IS the hub) or the cross-hub
    // dispatcher fans out across federated_peers (when the local
    // daemon is forwarding on behalf of a CLI bound to a peer
    // realm).
    if role == "hub" {
        if !node_id.is_empty() {
            bail!(
                "--node `{trimmed}` has trailing tail after /hub; URI v4.1.4 hub URI is bare \
                 (no id segment). Expected: easynet:///r/<realm>/hub. \
                 {URI_DISCOVERY_HINT}"
            );
        }
        if realm.is_empty() {
            bail!(
                "--node `{trimmed}` has empty realm. Expected: easynet:///r/<realm>/hub. \
                 {URI_DISCOVERY_HINT}"
            );
        }
        return Ok(trimmed.to_string());
    }

    if realm.is_empty()
        || node_id.is_empty()
        || node_id.contains('/')
        || (role == "device" && node_id.contains('.'))
        || !matches!(role, "device" | "agent")
    {
        bail!(
            "--node `{trimmed}` does not parse as easynet:///r/<realm>/device/<node-uuid>. \
             Got realm={realm:?}, segment={role:?}, node={node_id:?}. \
             {URI_DISCOVERY_HINT}"
        );
    }
    if role == "device" {
        return Ok(trimmed.to_string());
    }
    if node_id.contains('.') {
        bail!(
            "--node `{trimmed}` names an agent profile, not a device. \
             Cross-hub routing requires easynet:///r/<realm>/device/<node-uuid>. \
             {URI_DISCOVERY_HINT}"
        );
    }
    Ok(crate::uri::device_uri(realm, node_id))
}

/// Operator-actionable hint appended to every `--node` parse-
/// failure error. Names the four places a real URI can come from
/// today, ordered most-helpful-first. Centralised so the wording
/// stays byte-identical across both `parse_node_uri` failure
/// arms — operators can grep one substring across logs.
///
/// **PR-N1 user-flow review catch**: PR-N1 ships the CLI
/// invocation surface (commit 8/N) but cross-hub URI discovery
/// from a remote machine without manual config is PR-N3
/// territory (cross-realm directory federation, not yet shipped).
/// Until PR-N3 lands, operators construct URIs by hand from the
/// sources below; the error message points at them.
const URI_DISCOVERY_HINT: &str = "Where to find a canonical URI today (until PR-N3 cross-realm \
     directory federation ships): \
     (1) `cat ~/.easynet/credentials.json` on the target machine — \
     concat `easynet:///r/<tenant_id>/device/<node_id>` from the fields. \
     (2) `cat ~/.easynet/daemon-config.toml` and read the \
     `[daemon.federated_peers]` table — keys are tenant ids the \
     local daemon already trusts. \
     (3) `cat /etc/easynet/realm-trust.toml` and read \
     `[[trusted_agent]]` blocks with `role = \"hub\"` — those \
     are the peer hubs the cross-hub dialer can reach. \
     (4) `easynet ability invoke easynet.discover` — local-realm only \
     today; cross-realm enumeration ships in PR-N3.";

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

    // The daemon's `federation.forward_invoke` request shape per
    // DEC-N4 §2.1: `ForwardInvokeRequest { target_uri,
    // inner_envelope_b64, causal_context_bytes,
    // forward_deadline_ms }`. The inner envelope carries the
    // original `(ability, args, call_id)` tuple as a JSON blob;
    // PR-N1's daemon-to-daemon wire forwards this opaquely. The
    // outer audit / deadline fields round-trip verbatim so
    // PR-N5's InvocationReceipt can stamp causal_context.list and
    // DEC-N5 §3 can derive the inner deadline. PR-N2 will
    // introduce AXIOM mapping rewrite + signature; PR-N1 ships
    // the unsigned shape.
    //
    // DEC-N4 §2.1: a client-minted `call_id` is required so the
    // daemon's `ForwardInvokeResponse.correlation_call_id` can
    // thread it back to whichever bidi was waiting for the
    // result. We use a nanosecond+rng id; the value is opaque
    // to the daemon, only the round-trip equality matters.
    let call_id = generate_call_id();
    let inner_payload = json!({
        "ability": ability,
        "args": args,
        "call_id": call_id,
    });
    let inner_envelope_bytes = serde_json::to_vec(&inner_payload)
        .context("serialise inner ability call as forward_invoke payload")?;
    let inner_envelope_b64 = base64_engine_encode(&inner_envelope_bytes);

    // DEC-N4 §2.1 audit-chain + deadline fields. The CLI bridge
    // is the lowest-level synchronous initiator; it has no prior
    // `ForwardReceipt` to chain (those are PR-N5 territory) and
    // no caller-side deadline budget (CLI invocations run to
    // completion). Both fields ship as their zero-shape so the
    // peer hub treats them as "no caller hint, apply defaults".
    // Once `<self>.invoke_remote` initiator becomes the upstream
    // caller (instead of a direct CLI dial), it will populate
    // both with real values per PR-N5 §1 / DEC-N5 §3.
    let forward_args = json!({
        "target_uri": node_uri,
        "inner_envelope_b64": inner_envelope_b64,
        "causal_context_bytes": Vec::<u8>::new(),
        "forward_deadline_ms": 0_u64,
    });
    let forward_args_bytes =
        serde_json::to_vec(&forward_args).context("serialise ForwardInvokeRequest")?;

    // Build the outer envelope. The caller URI defaults to a
    // generic `easynet:///r/cli/device/local` so the daemon's
    // admission gate has something to log; production deployments
    // will override this with the operator's identity once
    // PR-N2 cross-realm signing lands.
    //
    // AXIOM §A1 requires both `caller` and `callee`; the daemon's
    // admission gate rejects an envelope missing either with
    // `AXON_AXIOM_ENVELOPE_INCOMPLETE:callee_missing`. For a
    // `federation.forward_invoke` call the inner ability's
    // `target_uri` is the ultimate addressee, so we stamp it as
    // both `callee` (intermediate routing hop addressee per A1
    // "intermediate hops MAY repeat target") and `subject` (the
    // identity the inner ability runs against). This matches the
    // backend Go side's `daemon_grpc/invoke_remote.go` envelope
    // convention.
    // v4.1.5 §A.URA-3 strict parsing: agent tail MUST be
    // `<user-uuid>.<agent-id>` (split on dot). The legacy
    // `r/cli/agent/local` fallback fails the strict parser
    // because `local` has no dot. Use the device shape instead —
    // `r/cli/device/local` parses cleanly (device tail is a bare
    // token with no dot/slash). The daemon's loopback bypass +
    // the Postel-permissive admission paths still admit either,
    // but we want the wire to ship a parseable URA so that any
    // caller-side strict validator (or a future enforce-mode
    // flip) does not reject the envelope.
    let resolved_caller_uri = caller_uri
        .unwrap_or("easynet:///r/cli/device/local")
        .to_string();
    // AXIOM §A1: `invocation_nonce` is a 16-byte random value the
    // daemon's admission gate uses to dedup replays
    // (`AXON_NONCE_REPLAY` rejects on a hit in the replay window).
    // CLI-initiated calls are one-shot, so a fresh `OsRng` 16-byte
    // sample per call is sufficient.
    use rand::RngCore;
    let mut invocation_nonce = vec![0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut invocation_nonce);
    let envelope = Envelope {
        caller: Some(AgentIdentity {
            uri: resolved_caller_uri,
            ..AgentIdentity::default()
        }),
        callee: Some(AgentIdentity {
            uri: node_uri.to_string(),
            ..AgentIdentity::default()
        }),
        subject: Some(SubjectIdentity {
            uri: node_uri.to_string(),
            ..SubjectIdentity::default()
        }),
        invocation_nonce,
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
                    let stream = tokio::net::UnixStream::connect(path).await?;
                    Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
                }
            }))
            .await
            .context("connect to local daemon gRPC UDS socket")?;

        let mut client = InvocationClient::new(channel);
        let response = client.invoke(request).await.map_err(|status| {
            // DEC-N4 §2.1: cross-hub `target_offline` surfaces
            // as `Status::failed_precondition` with reason text
            // `target_offline`. Translate that into an
            // operator-actionable bail (rather than a generic
            // RPC error) so a CLI user reading the message
            // knows the call reached the daemon but the peer
            // was unreachable.
            if status.code() == tonic::Code::FailedPrecondition
                && status.message().contains("target_offline")
            {
                anyhow!(
                    "cross-hub target `{node_uri}` is offline (federation.forward_invoke \
                     reported target_offline). Run `easynet federation peers` to see \
                     reachable peers, or `easynet runtime status` on the peer machine \
                     to confirm the daemon is running."
                )
            } else {
                anyhow!(
                    "daemon error invoking federation.forward_invoke for target `{node_uri}` \
                     (code={:?}): {}",
                    status.code(),
                    status.message(),
                )
            }
        })?;
        let body = response.into_inner();
        // DEC-N4 §2.1 final shape:
        // ForwardInvokeResponse { result_bytes, correlation_call_id }
        // is the wire surface. Parse + extract result_bytes.
        // For local-tenant fast-path the daemon returns
        // `result_bytes: empty` (the actual ability response
        // flows back over the reverse-channel correlation
        // path). For cross-tenant the result_bytes is the
        // peer's full ability response; if it parses as JSON
        // we hand the parsed value back, otherwise the raw
        // bytes (lossy decoded as UTF-8) so the CLI can still
        // print something useful.
        let envelope: Value = serde_json::from_slice(&body.result).with_context(|| {
            format!(
                "parse federation.forward_invoke ForwardInvokeResponse envelope: \
                 result_content_type={:?}",
                body.result_content_type
            )
        })?;
        let result_bytes_b64 = envelope
            .get("result_bytes")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|n| n.as_u64())
                    .map(|n| n as u8)
                    .collect::<Vec<u8>>()
            })
            .unwrap_or_default();
        if result_bytes_b64.is_empty() {
            // Local-tenant fast-path delivery accepted, or a
            // cross-hub call where the peer ability genuinely
            // returned empty bytes. Either way, the
            // correlation id is the only useful surface to
            // print.
            let correlation = envelope
                .get("correlation_call_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            return Ok(serde_json::json!({
                "delivery": "accepted",
                "correlation_call_id": correlation,
            }));
        }
        // Try parsing the peer's ability response as JSON; if
        // it isn't, fall back to a hex-stringified shape so
        // the CLI's print path doesn't crash on binary
        // payloads.
        match serde_json::from_slice::<Value>(&result_bytes_b64) {
            Ok(v) => Ok(v),
            Err(_) => Ok(serde_json::json!({
                "result_bytes_len": result_bytes_b64.len(),
                "result_bytes_hex": result_bytes_b64
                    .iter()
                    .map(|b| format!("{:02x}", b))
                    .collect::<String>(),
            })),
        }
    })
}

/// Cross-realm directory query against the local daemon's
/// `federation.discover` ability. Returns the parsed
/// `entries: [DirectoryEntry]` array verbatim. The single
/// dial path (UDS gRPC InvocationClient → daemon's
/// `dispatch_federation_discover`) is shared with
/// `easynet federation discover`'s own subcommand — having
/// one helper means `device list` / `auth devices` / future
/// fan-out callers cannot drift on caller URI / envelope
/// shape.
///
/// Args:
///   * `agent_uri_filter` — optional URI filter passed verbatim
///     to the daemon. `None` returns the full federated
///     directory; `Some(uri)` returns at most one entry (lex
///     tie-break on peer realm).
///   * `caller_uri` — optional caller URI for the envelope. When
///     `None`, falls back to the device URI minted from
///     `credentials.json`, then the generic
///     `easynet:///r/cli/device/local` placeholder. The daemon's
///     loopback bypass admits both shapes.
///
/// Returns the `entries` array as a `Vec<Value>` (each element
/// is a `DirectoryEntry`-shaped JSON object).
pub fn invoke_federation_discover(
    agent_uri_filter: Option<&str>,
    caller_uri: Option<&str>,
) -> anyhow::Result<Vec<Value>> {
    let socket_path = expand_home(DEFAULT_DAEMON_GRPC_UDS_PATH);
    if !socket_path.exists() {
        bail!(
            "daemon not running (no gRPC socket at {}). \
             Start it with `easynet runtime start`.",
            socket_path.display()
        );
    }

    let mut req_args = json!({});
    if let Some(uri) = agent_uri_filter {
        req_args["agent_uri"] = Value::String(uri.to_string());
    }
    let arg_bytes = serde_json::to_vec(&req_args).context("encode discover args")?;

    let resolved_caller = caller_uri
        .map(str::to_string)
        .or_else(|| {
            crate::persistence::config::load_credentials()
                .ok()
                .map(|c| crate::uri::device_uri(&c.tenant_id, &c.node_id))
        })
        .unwrap_or_else(|| crate::uri::device_uri("cli", "local"));

    let envelope = Envelope {
        caller: Some(AgentIdentity {
            uri: resolved_caller.clone(),
            ..AgentIdentity::default()
        }),
        callee: Some(AgentIdentity {
            uri: resolved_caller.clone(),
            ..AgentIdentity::default()
        }),
        subject: Some(SubjectIdentity {
            uri: resolved_caller,
            ..SubjectIdentity::default()
        }),
        ..Envelope::default()
    };

    let request = InvokeRequest {
        envelope: Some(envelope),
        function_name: "federation.discover".to_string(),
        arguments: arg_bytes,
        ..InvokeRequest::default()
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime for federation.discover")?;

    let response = runtime.block_on(async move {
        let socket = socket_path.clone();
        let endpoint = Endpoint::try_from("http://[::1]:50051")
            .context("build tonic endpoint")?
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5));
        let channel = endpoint
            .connect_with_connector(service_fn(move |_: Uri| {
                let path = socket.clone();
                async move {
                    let stream = tokio::net::UnixStream::connect(path).await?;
                    Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
                }
            }))
            .await
            .context("connect to local daemon gRPC UDS")?;
        let mut client = InvocationClient::new(channel);
        let resp = client.invoke(request).await.map_err(|status| {
            anyhow!(
                "daemon rejected federation.discover: code={:?} message={}",
                status.code(),
                status.message()
            )
        })?;
        Ok::<_, anyhow::Error>(resp.into_inner())
    })?;

    let body: Value =
        serde_json::from_slice(&response.result).context("decode discover response body")?;
    Ok(body
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
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

/// Generate a fresh `call_id` for the inner envelope payload's
/// DEC-N4 §2.1 correlation field. Format: `cli-<nanos-hex>` —
/// nanoseconds since Unix epoch, hex-encoded, prefixed so log
/// scraping can tell CLI-minted ids apart from daemon-minted
/// ones (`<self>.invoke_remote` initiator path uses a different
/// prefix). Collision space is `2^64` per second of clock
/// resolution; for the CLI's typical hand-driven cadence the
/// risk is negligible.
fn generate_call_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("cli-{nanos:x}")
}

/// Minimal base64 encoder used for the inner envelope payload.
/// Avoids pulling `base64` as a top-level dep just for one
/// call site; the inner envelope is a CLI-internal blob, not a
/// wire-stable string, so a per-call encoder is fine. Matches
/// the standard alphabet so a daemon-side decoder using the
/// `base64` crate would interoperate.
fn base64_engine_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
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
            parse_node_uri("easynet:///r/realm-b/device/laptop-bob").expect("canonical accepted");
        assert_eq!(parsed, "easynet:///r/realm-b/device/laptop-bob");
    }

    #[test]
    fn parse_node_uri_accepts_hub_uri_for_cross_hub_calls() {
        // URI v4.1.4: bare /hub URI is the realm's singleton hub
        // identifier — used as `--node` target for cross-hub
        // federation.heartbeat / federation.* calls.
        let parsed =
            parse_node_uri("easynet:///r/peer-realm/hub").expect("v4.1.4 hub URI accepted");
        assert_eq!(parsed, "easynet:///r/peer-realm/hub");
    }

    #[test]
    fn parse_node_uri_rejects_hub_with_trailing_id() {
        // /hub is a complete URA on its own; trailing tail
        // indicates v1 `agent/01HUB`-style mistake.
        let err = parse_node_uri("easynet:///r/realm/hub/01HUB").expect_err("trailing id rejected");
        assert!(
            err.to_string().contains("trailing tail"),
            "expected trailing-tail diagnostic, got: {err}"
        );
    }

    #[test]
    fn parse_node_uri_trims_surrounding_whitespace() {
        let parsed =
            parse_node_uri("  easynet:///r/realm-b/device/n1  \n").expect("whitespace trimmed");
        assert_eq!(parsed, "easynet:///r/realm-b/device/n1");
    }

    #[test]
    fn parse_node_uri_normalises_legacy_agent_device_shape() {
        let parsed =
            parse_node_uri("easynet:///r/realm-b/agent/n1").expect("legacy device accepted");
        assert_eq!(parsed, "easynet:///r/realm-b/device/n1");
    }

    #[test]
    fn parse_node_uri_rejects_https_url() {
        let err = parse_node_uri("https://hub.example/r/foo").expect_err("https rejected");
        assert!(
            format!("{err}").contains("not a canonical EasyNet device URI"),
            "error message must cite the canonical-URI requirement, got: {err}"
        );
    }

    #[test]
    fn parse_node_uri_rejects_bare_hostname() {
        let err = parse_node_uri("hub.example.com").expect_err("bare hostname rejected");
        assert!(format!("{err}").contains("canonical"));
    }

    #[test]
    fn parse_node_uri_rejects_missing_tenant() {
        let err = parse_node_uri("easynet:///r//device/n1").expect_err("empty tenant rejected");
        assert!(
            format!("{err}").contains("does not parse"),
            "must surface a structural-mismatch error, got: {err}"
        );
    }

    #[test]
    fn parse_node_uri_rejects_missing_node_id() {
        let err =
            parse_node_uri("easynet:///r/realm-b/device/").expect_err("empty node id rejected");
        assert!(format!("{err}").contains("does not parse"));
    }

    #[test]
    fn parse_node_uri_rejects_extra_path_segments() {
        let err = parse_node_uri("easynet:///r/realm-b/device/n1/ability/x")
            .expect_err("extra segments rejected");
        assert!(format!("{err}").contains("does not parse"));
    }

    #[test]
    fn parse_node_uri_rejects_real_agent_profile_uri() {
        let err = parse_node_uri("easynet:///r/realm-b/agent/alice.claude")
            .expect_err("hosted agent must not be accepted");
        assert!(format!("{err}").contains("agent profile"));
    }

    #[test]
    fn parse_node_uri_rejects_wrong_keyword() {
        // URI v4.1.4: the role segment after the realm MUST be
        // either `device` (canonical) or `agent` (v1 compat
        // window). A typo / unknown role is rejected so the
        // operator notices before the URI hits the daemon.
        let err =
            parse_node_uri("easynet:///r/realm-b/notarole/n1").expect_err("wrong keyword rejected");
        assert!(format!("{err}").contains("does not parse"));
    }

    #[test]
    fn parse_node_uri_failure_message_includes_discovery_hint() {
        // PR-N1 user-flow review catch: operators have no
        // zero-config way to discover a peer's canonical URI today
        // (PR-N3 territory). Until PR-N3 ships, the error message
        // tells them where to look — credentials.json, daemon-
        // config.toml's federated_peers table, /etc/easynet/realm-
        // trust.toml's hub entries, easynet.discover (local-realm).
        // Both rejection arms (non-easynet scheme + structural
        // mismatch) MUST cite the discovery hint so a typo'd
        // command surfaces the same operator-actionable next step.
        let err_scheme = parse_node_uri("not-an-easynet-uri").expect_err("rejected");
        let msg_scheme = format!("{err_scheme}");
        assert!(
            msg_scheme.contains("credentials.json"),
            "scheme-arm error must cite credentials.json discovery path; got: {msg_scheme}"
        );
        assert!(
            msg_scheme.contains("daemon-config.toml"),
            "scheme-arm error must cite daemon-config.toml discovery path; got: {msg_scheme}"
        );
        assert!(
            msg_scheme.contains("realm-trust.toml"),
            "scheme-arm error must cite realm-trust.toml discovery path; got: {msg_scheme}"
        );
        assert!(
            msg_scheme.contains("easynet.discover"),
            "scheme-arm error must cite easynet.discover ability; got: {msg_scheme}"
        );

        // Structural-failure: empty realm tail. The role segment
        // accepts both `device` (v4.1.4 canonical) and legacy bare
        // `agent` tails, so use a clearly malformed input — empty
        // realm — to exercise the structural-arm error path.
        let err_struct = parse_node_uri("easynet:///r//device/n1").expect_err("rejected");
        let msg_struct = format!("{err_struct}");
        assert!(
            msg_struct.contains("credentials.json") && msg_struct.contains("daemon-config.toml"),
            "structural-arm error must cite the same discovery hint; got: {msg_struct}"
        );
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
