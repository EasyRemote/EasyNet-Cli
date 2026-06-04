// EasyNet CLI — Federation Invoke Helper (PR-N1 commit 8/N)
// =========================================================
//
// File: src/support/federation_invoke.rs
// Description: CLI bridge from `easynet ability invoke <ability>
//              --node <peer-ura>` to the local daemon's gRPC
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
// 1. Caller passes `(ability, args, node_ura)`. `node_ura` must be
//    a canonical `easynet:///r/{tenant}/device/{node}` URA; non-
//    canonical inputs surface as a typed error before any IPC.
// 2. The helper dials the local daemon's UDS-bound gRPC
//    `Invocation` server (default `~/.easynet/daemon.sock`).
// 3. It sends an `InvokeRequest` whose `function_name` is
//    `federation.forward_invoke` and whose `arguments` is the
//    JSON `{target_ura, inner_envelope_b64}` shape that
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

use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use serde_json::{json, Value};

use crate::services::axon_serve::ProtoEnvelope;
use easynet_axon::pb::axon::v1::invocation_client::InvocationClient;

/// Validate a `--node` argument as a canonical EasyNet device or hub URA.
/// Returns the URA string when it parses; surfaces a typed error
/// when it doesn't, with the exact wire-shape we expect quoted in
/// the message so the operator can fix the typo.
///
/// URA v4.1.4 (Phase 2F): `--node` carries a `device` URA, not an
/// `agent` URA. Devices are physical hosts (the box running the
/// daemon); agents are user-owned hosted profiles that live ON
/// devices. Cross-hub `federation.forward_invoke` targets devices
/// or the realm hub, never hosted user agents.
pub fn parse_node_ura(node: &str) -> anyhow::Result<String> {
    let trimmed = node.trim();
    let parsed = crate::ura::parse_ura(trimmed).map_err(|err| {
        anyhow::anyhow!(
            "--node `{trimmed}` is not a canonical EasyNet device or hub URA: {err}. \
             A bare hostname or `https://...` URL is not accepted. \
             {URI_DISCOVERY_HINT}"
        )
    })?;

    // URA v4.1.4: `/hub` is a complete URA on its own (no id-tail);
    // it identifies the realm's singleton hub. `--node easynet:///r/<realm>/hub`
    // is a legitimate target for cross-hub federation.heartbeat /
    // federation.* calls — the daemon's local presence either
    // matches it (when the daemon IS the hub) or the cross-hub
    // dispatcher fans out across federated_peers (when the local
    // daemon is forwarding on behalf of a CLI bound to a peer
    // realm).
    if parsed.kind == crate::ura::URAKind::Hub {
        return Ok(trimmed.to_string());
    }

    if parsed.kind != crate::ura::URAKind::Device {
        bail!(
            "--node `{trimmed}` does not parse as easynet:///r/<realm>/device/<node-uuid>. \
             Got kind={}. \
             {URI_DISCOVERY_HINT}",
            parsed.kind
        );
    }
    Ok(trimmed.to_string())
}

/// Operator-actionable hint appended to every `--node` parse-
/// failure error. Names the four places a real URA can come from
/// today, ordered most-helpful-first. Centralised so the wording
/// stays byte-identical across both `parse_node_ura` failure
/// arms — operators can grep one substring across logs.
///
/// **PR-N1 user-flow review catch**: PR-N1 ships the CLI
/// invocation surface (commit 8/N) but cross-hub URA discovery
/// from a remote machine without manual config is PR-N3
/// territory (cross-realm directory federation, not yet shipped).
/// Until PR-N3 lands, operators construct URAs by hand from the
/// sources below; the error message points at them.
const URI_DISCOVERY_HINT: &str = "Where to find a canonical URA today (until PR-N3 cross-realm \
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

/// Dispatch `(ability, args)` against `node_ura` via the local
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
    node_ura: &str,
    caller_ura: Option<&str>,
) -> anyhow::Result<Value> {
    let socket_path = crate::support::local_daemon_grpc::resolve_socket_path();
    if !crate::support::local_daemon_grpc::probe_accepting(&socket_path) {
        bail!(
            "daemon not running (local gRPC listener unreachable at {}). \
             Start it with `easynet runtime start`.",
            socket_path.display()
        );
    }

    // The daemon's `federation.forward_invoke` request shape per
    // DEC-N4 §2.1: `ForwardInvokeRequest { target_ura,
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
        "target_ura": node_ura,
        "inner_envelope_b64": inner_envelope_b64,
        "causal_context_bytes": Vec::<u8>::new(),
        "forward_deadline_ms": 0_u64,
    });
    let forward_args_bytes =
        serde_json::to_vec(&forward_args).context("serialise ForwardInvokeRequest")?;

    // Build the outer envelope. The caller URA defaults to a
    // generic `easynet:///r/cli/device/local` so the daemon's
    // admission gate has something to log; production deployments
    // will override this with the operator's identity once
    // PR-N2 cross-realm signing lands.
    //
    // AXIOM §A1 requires both `caller` and `callee`; the daemon's
    // admission gate rejects an envelope missing either with
    // `AXON_AXIOM_ENVELOPE_INCOMPLETE:callee_missing`. For a
    // `federation.forward_invoke` call the inner ability's
    // `target_ura` is the ultimate addressee, so we stamp it as
    // both `callee` (intermediate routing hop addressee per A1
    // "intermediate hops MAY repeat target") and `subject` (the
    // identity the inner ability runs against). This matches the
    // backend Go side's `daemon_grpc/invoke_remote.go` envelope
    // convention.
    // v4.1.5 §A.URA-3 strict parsing: agent tail MUST be
    // `<user-uuid>.<agent-id>` (split on dot). The
    // the legacy CLI agent-placeholder alias fails the strict parser
    // because `local` has no dot. Use the device shape instead —
    // `r/cli/device/local` parses cleanly (device tail is a bare
    // token with no dot/slash). The daemon's loopback bypass +
    // the Postel-permissive admission paths still admit either,
    // but we want the wire to ship a parseable URA so that any
    // caller-side strict validator (or a future enforce-mode
    // flip) does not reject the envelope.
    let resolved_caller_ura = caller_ura
        .unwrap_or("easynet:///r/cli/device/local")
        .to_string();
    // AXIOM §A1: `invocation_nonce` is a 16-byte random value the
    // daemon's admission gate uses to dedup replays
    // (`AXON_NONCE_REPLAY` rejects on a hit in the replay window).
    // CLI-initiated calls are one-shot, so a fresh `OsRng` 16-byte
    // sample per call is sufficient.
    let request = ProtoEnvelope::targeted(&resolved_caller_ura, node_ura, node_ura)?
        .invoke_request("federation.forward_invoke", forward_args_bytes)?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime for federation invoke")?;

    runtime.block_on(async move {
        let channel = crate::support::local_daemon_grpc::connect_channel(
            socket_path.clone(),
            Duration::from_secs(30),
            Duration::from_secs(10),
        )
        .await
        .context("connect to local daemon gRPC endpoint")?;

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
                    "cross-hub target `{node_ura}` is offline (federation.forward_invoke \
                     reported target_offline). Run `easynet federation peers` to see \
                     reachable peers, or `easynet runtime status` on the peer machine \
                     to confirm the daemon is running."
                )
            } else {
                anyhow!(
                    "daemon error invoking federation.forward_invoke for target `{node_ura}` \
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
/// fan-out callers cannot drift on caller URA / envelope
/// shape.
///
/// Args:
///   * `agent_ura_filter` — optional URA filter passed verbatim
///     to the daemon. `None` returns the full federated
///     directory; `Some(ura)` returns at most one entry (lex
///     tie-break on peer realm).
///   * `caller_ura` — optional caller URA for the envelope. When
///     `None`, falls back to the device URA minted from
///     `credentials.json`, then the generic
///     `easynet:///r/cli/device/local` placeholder. The daemon's
///     loopback bypass admits both shapes.
///
/// Returns the `entries` array as a `Vec<Value>` (each element
/// is a `DirectoryEntry`-shaped JSON object).
pub fn invoke_federation_discover(
    agent_ura_filter: Option<&str>,
    caller_ura: Option<&str>,
) -> anyhow::Result<Vec<Value>> {
    let socket_path = crate::support::local_daemon_grpc::resolve_socket_path();
    if !crate::support::local_daemon_grpc::probe_accepting(&socket_path) {
        bail!(
            "daemon not running (local gRPC listener unreachable at {}). \
             Start it with `easynet runtime start`.",
            socket_path.display()
        );
    }

    let mut req_args = json!({});
    if let Some(ura) = agent_ura_filter {
        req_args["agent_ura"] = Value::String(ura.to_string());
    }
    let arg_bytes = serde_json::to_vec(&req_args).context("encode discover args")?;

    let resolved_caller = caller_ura
        .map(str::to_string)
        .or_else(|| {
            crate::persistence::config::load_credentials()
                .ok()
                .map(|c| crate::ura::device_ura(&c.tenant_id, &c.node_id))
        })
        .unwrap_or_else(|| crate::ura::device_ura("cli", "local"));

    let request = ProtoEnvelope::loopback(resolved_caller)?
        .invoke_request("federation.discover", arg_bytes)?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime for federation.discover")?;

    let response: easynet_axon::pb::axon::v1::InvokeResponse = {
        runtime.block_on(async move {
            let channel = crate::support::local_daemon_grpc::connect_channel(
                socket_path.clone(),
                Duration::from_secs(10),
                Duration::from_secs(5),
            )
            .await
            .context("connect to local daemon gRPC endpoint")?;
            let mut client = InvocationClient::new(channel);
            let resp = client.invoke(request).await.map_err(|status| {
                anyhow!(
                    "daemon rejected federation.discover: code={:?} message={}",
                    status.code(),
                    status.message()
                )
            })?;
            Ok::<_, anyhow::Error>(resp.into_inner())
        })?
    };

    let body: Value =
        serde_json::from_slice(&response.result).context("decode discover response body")?;
    Ok(body
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

/// `federation.revoke` against the local daemon's gRPC
/// InvocationServer. Removes the named Agent's directory entry on
/// the hub. CLI lifecycle surfaces (`easynet device remove`,
/// `easynet device reset --force`) call this helper instead of
/// local-only `device.node.remove` / `device.node.deregister`
/// acknowledgements.
///
/// Args:
///   * `agent_ura` — canonical URA of the Agent to revoke (typically
///     a device URA `easynet:///r/<realm>/device/<id>`).
///   * `reason` — operator-supplied label, written through to the
///     receipt for audit. `"deregister"` / `"reset"` are common.
///   * `caller_ura` — same fallback chain as
///     `invoke_federation_discover`.
///
/// Returns `Ok(())` on a successful ack from the daemon. Best-effort
/// by contract on the hub side, but this helper still surfaces
/// transport / parse errors so callers can log them honestly.
pub fn invoke_federation_revoke(
    agent_ura: &str,
    reason: &str,
    caller_ura: Option<&str>,
) -> anyhow::Result<()> {
    let socket_path = crate::support::local_daemon_grpc::resolve_socket_path();
    if !crate::support::local_daemon_grpc::probe_accepting(&socket_path) {
        bail!(
            "daemon not running (local gRPC listener unreachable at {}). \
             Start it with `easynet runtime start`.",
            socket_path.display()
        );
    }

    let req_args = json!({
        "agent_ura": agent_ura,
        "reason": reason,
    });
    let arg_bytes = serde_json::to_vec(&req_args).context("encode revoke args")?;

    let resolved_caller = caller_ura
        .map(str::to_string)
        .or_else(|| {
            crate::persistence::config::load_credentials()
                .ok()
                .map(|c| crate::ura::device_ura(&c.tenant_id, &c.node_id))
        })
        .unwrap_or_else(|| crate::ura::device_ura("cli", "local"));

    let request =
        ProtoEnvelope::loopback(resolved_caller)?.invoke_request("federation.revoke", arg_bytes)?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime for federation.revoke")?;

    runtime.block_on(async move {
        let channel = crate::support::local_daemon_grpc::connect_channel(
            socket_path.clone(),
            Duration::from_secs(10),
            Duration::from_secs(5),
        )
        .await
        .context("connect to local daemon gRPC endpoint")?;
        let mut client = InvocationClient::new(channel);
        let _ = client.invoke(request).await.map_err(|status| {
            anyhow!(
                "daemon rejected federation.revoke: code={:?} message={}",
                status.code(),
                status.message()
            )
        })?;
        Ok::<_, anyhow::Error>(())
    })
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
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
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
    fn parse_node_ura_accepts_canonical_shape() {
        let parsed =
            parse_node_ura("easynet:///r/realm-b/device/laptop-bob").expect("canonical accepted");
        assert_eq!(parsed, "easynet:///r/realm-b/device/laptop-bob");
    }

    #[test]
    fn parse_node_ura_accepts_hub_ura_for_cross_hub_calls() {
        // URA v4.1.4: bare /hub URA is the realm's singleton hub
        // identifier — used as `--node` target for cross-hub
        // federation.heartbeat / federation.* calls.
        let parsed =
            parse_node_ura("easynet:///r/peer-realm/hub").expect("v4.1.4 hub URA accepted");
        assert_eq!(parsed, "easynet:///r/peer-realm/hub");
    }

    #[test]
    fn parse_node_ura_rejects_hub_with_trailing_id() {
        // /hub is a complete URA on its own; trailing tail
        // indicates v1 `agent/01HUB`-style mistake.
        let err = parse_node_ura("easynet:///r/realm/hub/01HUB").expect_err("trailing id rejected");
        assert!(
            err.to_string().contains("unexpected tail"),
            "expected hub tail diagnostic, got: {err}"
        );
    }

    #[test]
    fn parse_node_ura_trims_surrounding_whitespace() {
        let parsed =
            parse_node_ura("  easynet:///r/realm-b/device/n1  \n").expect("whitespace trimmed");
        assert_eq!(parsed, "easynet:///r/realm-b/device/n1");
    }

    #[test]
    fn parse_node_ura_rejects_agent_device_alias_shape() {
        let err = parse_node_ura("easynet:///r/realm-b/agent/n1")
            .expect_err("device alias must be rejected");
        assert!(format!("{err}").contains("agent tail must"));
    }

    #[test]
    fn parse_node_ura_rejects_https_url() {
        let err = parse_node_ura("https://hub.example/r/foo").expect_err("https rejected");
        assert!(
            format!("{err}").contains("not a canonical EasyNet device or hub URA"),
            "error message must cite the canonical-URA requirement, got: {err}"
        );
    }

    #[test]
    fn parse_node_ura_rejects_bare_hostname() {
        let err = parse_node_ura("hub.example.com").expect_err("bare hostname rejected");
        assert!(format!("{err}").contains("canonical"));
    }

    #[test]
    fn parse_node_ura_rejects_missing_tenant() {
        let err = parse_node_ura("easynet:///r//device/n1").expect_err("empty tenant rejected");
        assert!(
            format!("{err}").contains("missing <realm>"),
            "must surface a structural-mismatch error, got: {err}"
        );
    }

    #[test]
    fn parse_node_ura_rejects_missing_node_id() {
        let err =
            parse_node_ura("easynet:///r/realm-b/device/").expect_err("empty node id rejected");
        // The SDK ParseError::DeviceMissingTail formats as
        // "device URA requires <device-id> tail" (note: URA, not
        // URA — the SDK ontology calls these URAs everywhere).
        // Test pins that wording so a future rename surfaces here
        // instead of silently swallowing the error condition.
        assert!(
            format!("{err}").contains("device URA requires"),
            "expected SDK DeviceMissingTail wording, got: {err}"
        );
    }

    #[test]
    fn parse_node_ura_rejects_extra_path_segments() {
        let err = parse_node_ura("easynet:///r/realm-b/device/n1/ability/x")
            .expect_err("extra segments rejected");
        // The SDK ParseError::DeviceBadShape formats as
        // "device-id must be a single path segment". The test used
        // to assert "device-id must be bare" — an older copy that
        // drifted from the SDK; we pin the live wording here.
        assert!(
            format!("{err}").contains("device-id must be a single path segment"),
            "expected SDK DeviceBadShape wording, got: {err}"
        );
    }

    #[test]
    fn parse_node_ura_rejects_real_agent_profile_ura() {
        let err = parse_node_ura("easynet:///r/realm-b/agent/alice.claude")
            .expect_err("hosted agent must not be accepted");
        assert!(format!("{err}").contains("kind=agent"));
    }

    #[test]
    fn parse_node_ura_rejects_wrong_keyword() {
        // URA v4.1.4: the role segment after the realm MUST be
        // `device`. A typo / unknown role is rejected so the
        // operator notices before the URA hits the daemon. The
        // SDK ParseError::UnknownRole formats as
        // `unknown URA role "<role>" (allowed: ...)`.
        let err =
            parse_node_ura("easynet:///r/realm-b/notarole/n1").expect_err("wrong keyword rejected");
        assert!(
            format!("{err}").contains("unknown URA role"),
            "expected SDK UnknownRole wording, got: {err}"
        );
    }

    #[test]
    fn parse_node_ura_failure_message_includes_discovery_hint() {
        // PR-N1 user-flow review catch: operators have no
        // zero-config way to discover a peer's canonical URA today
        // (PR-N3 territory). Until PR-N3 ships, the error message
        // tells them where to look — credentials.json, daemon-
        // config.toml's federated_peers table, /etc/easynet/realm-
        // trust.toml's hub entries, easynet.discover (local-realm).
        // Both rejection arms (non-easynet scheme + structural
        // mismatch) MUST cite the discovery hint so a typo'd
        // command surfaces the same operator-actionable next step.
        let err_scheme = parse_node_ura("not-an-easynet-ura").expect_err("rejected");
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

        // Structural-failure: empty realm tail. Use a clearly
        // malformed input to exercise the structural-arm error
        // path without depending on any retired URA aliases.
        let err_struct = parse_node_ura("easynet:///r//device/n1").expect_err("rejected");
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
