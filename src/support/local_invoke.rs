// EasyNet CLI — Local ability invocation helper
// ==============================================
//
// File: src/support/local_invoke.rs
// Description: One function — `invoke_local_ability(name, args)` —
//              that every CLI subcommand uses to dispatch through
//              the local daemon's Axon Invocation gRPC surface
//              (~/.easynet/daemon.sock).
//
// Why this exists
// ---------------
// AXON-RFC-001 collapses every former "command" surface to one
// primitive: `Invoke <ability>`. Following that ontology in the
// CLI means each subcommand should be a thin wrapper that:
//
//   1. Maps the user's CLI args into a JSON args object.
//   2. Calls the appropriate ability via this helper.
//   3. Prints the result.
//
// Any subcommand that bypasses this — calling a transport
// directly, or constructing its own IPC client — is a layering
// violation: it ties the CLI to a specific transport (the
// federation bridge in pre-P1.5 code; an alternate IPC in some
// future variant) instead of to the ability surface. One helper
// here means one point to swap when the transport evolves.
//
// Routing model
// -------------
// Always local: the CLI is a thin client to the local daemon. A
// command that semantically needs a remote node (e.g. "show this
// device's siblings on the federation") must reach those nodes by
// invoking a federation-tier ability *on the local daemon*; the
// daemon is the only entity that holds federation transport
// state. The local-IPC contract here never grows a `--node` knob —
// federation routing belongs inside the ability, not in the CLI's
// dispatch path.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde_json::Value;

pub use crate::runtime::invocation_target::LocalAbilityTarget;

/// One decoded frame from a daemon-hosted server-stream ability.
///
/// This is the CLI/support-layer projection of Axon's
/// `InvokeStreamChunk`: transport metadata stays visible, while the
/// business payload is decoded to JSON for frontend and script
/// consumers. It is not a live subscription handle; callers receive
/// a finite vector only after the helper has drained until terminal
/// or an explicit frame limit.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalStreamFrame {
    /// Zero-based frame sequence assigned by the daemon transport.
    pub sequence: u64,
    /// Content type advertised by the daemon for this frame.
    pub content_type: String,
    /// Whether this frame is terminal for the stream.
    pub terminal: bool,
    /// Decoded JSON business payload. Empty payloads decode to null.
    pub payload: Value,
}

/// Typed failure classes for local-daemon invocation (F-023).
///
/// Minted at the transport layer where the cause is structurally known
/// (socket probe failed, crate built without `axon-pb`), so consumers
/// branch with [`classify_invoke_error`] instead of sniffing message
/// text.
#[derive(Debug, thiserror::Error)]
pub enum LocalInvokeFailure {
    /// The daemon is not reachable (listener probe failed, or this
    /// build has no gRPC transport). Falling back to an in-process
    /// executor is legitimate — nothing ran.
    #[error("{0}")]
    DaemonOffline(String),
}

/// Consumer-facing classification of a local-invoke error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalInvokeErrorKind {
    DaemonOffline,
    AbilityUnregistered,
    /// The daemon executed the request and it failed for real.
    /// Re-running through another executor would double-execute a
    /// side-effecting ability to mask a true error.
    Failed,
}

/// Classify a local-invoke error for fallback decisions.
///
/// Prefers the typed [`LocalInvokeFailure`] payload (walks the anyhow
/// chain). The string table below is the TRANSITIONAL fallback for
/// error paths that cannot mint typed payloads yet — daemon-side
/// status codes have no typed surface (RFC gap: flagged, not
/// extrapolated). It is the single permitted sniffing point in the
/// crate; consumers must not grow their own.
pub fn classify_invoke_error(err: &anyhow::Error) -> LocalInvokeErrorKind {
    for cause in err.chain() {
        if let Some(f) = cause.downcast_ref::<LocalInvokeFailure>() {
            return match f {
                LocalInvokeFailure::DaemonOffline(_) => LocalInvokeErrorKind::DaemonOffline,
            };
        }
    }
    let lower = format!("{err:#}").to_ascii_lowercase();
    if lower.contains("daemon not running")
        || lower.contains("listener unreachable")
        || lower.contains("connect to local axon daemon")
        || lower.contains("requires the `axon-pb` feature")
    {
        return LocalInvokeErrorKind::DaemonOffline;
    }
    if lower.contains("unknown_ability")
        || lower.contains("not_found")
        || lower.contains("no local handler registered")
    {
        return LocalInvokeErrorKind::AbilityUnregistered;
    }
    LocalInvokeErrorKind::Failed
}

/// Invoke an ability against the local daemon's Axon runtime.
///
/// `ability` is the wire-level qualified name (e.g. `easynet.discover`,
/// `claude.weather`, `observe.health`). `args` is forwarded as-is —
/// the helper does not validate the shape; the daemon-side handler
/// is the authority on argument validation, and a CLI-side
/// pre-check would only drift.
///
/// On success returns the raw value (whatever shape the handler
/// produced). On error returns a typed `anyhow::Error` with the
/// daemon-side `code` + `message` rendered into the message — the
/// CLI's outer layer can surface that verbatim or pattern-match if
/// it needs typed handling.
///
/// **Canonical entry point for the "one CLI subcommand = one
/// ability invoke" contract.** CLI surfaces MUST go through this
/// function (or [`invoke_local_ability_with_subject`]), not the
/// transport-level free fns in `support::local_daemon_grpc`. The
/// indirection looks redundant — the body is one line — but it
/// matters: the day the local-ability transport evolves, this is
/// the **one** call site that knows the underlying transport.
/// Callers that bypass it become per-surface transport coupling.
pub fn invoke_local_ability(ability: &str, args: Value) -> anyhow::Result<Value> {
    invoke_local_ability_with_subject(ability, args, None)
}

/// Same as [`invoke_local_ability`] but threads an optional
/// envelope subject through to the daemon. The subject lands in
/// `EnvelopeContext.subject` for handlers that consume it
/// (e.g. `camera.snapshot`, which routes its frame from the
/// resource the subject URI names).
pub fn invoke_local_ability_with_subject(
    ability: &str,
    args: Value,
    subject: Option<String>,
) -> anyhow::Result<Value> {
    crate::support::local_daemon_grpc::invoke_local_daemon_ability_with_subject(
        ability, args, subject,
    )
}

/// Invoke a canonical local Ability URA target through the daemon.
///
/// This path preserves the full descriptor owner identity in the signed
/// envelope. Use it for user-facing `ability invoke <ability-ura>` surfaces;
/// use the string-only helper only for daemon-owned system surfaces whose
/// callee really is the local device.
pub fn invoke_local_ability_target_with_subject_timeout(
    target: &LocalAbilityTarget,
    args: Value,
    subject: Option<String>,
    timeout: std::time::Duration,
) -> anyhow::Result<Value> {
    crate::support::local_daemon_grpc::invoke_local_daemon_ability_targeted_timeout(
        target.dispatch_name(),
        args,
        target.callee_ura(),
        target.default_subject_ura(),
        subject,
        timeout,
    )
}

/// Stream a canonical local Ability URA target through the daemon.
///
/// This is the stream-mode twin of
/// [`invoke_local_ability_target_with_subject_timeout`]; it keeps callee and
/// default subject tied to the canonical Ability owner instead of defaulting
/// them to the local device signer.
pub fn invoke_local_ability_target_stream_with_subject(
    target: &LocalAbilityTarget,
    args: Value,
    subject: Option<String>,
    timeout: std::time::Duration,
    max_frames: Option<usize>,
) -> anyhow::Result<Vec<LocalStreamFrame>> {
    crate::support::local_daemon_grpc::invoke_local_daemon_ability_targeted_stream_with_subject(
        target.dispatch_name(),
        args,
        target.callee_ura(),
        target.default_subject_ura(),
        subject,
        timeout,
        max_frames,
    )
}

/// Same as [`invoke_local_ability_with_subject`] but returns the
/// invocation record alongside the result.
///
/// This is the EAL mission runner's lowering surface: each mission
/// step becomes one complete seven-tuple Axon invocation. The
/// returned metadata value carries the envelope echo (caller /
/// callee / ability / subject / nonce / causal_context) plus the
/// ledger-assigned `invocation_ura`, `trace_id`, and receipt-chain
/// anchors — the material a downstream step needs to name THIS step
/// as its causal parent. `causal_parents` entries are
/// `{node, invocation_ura, receipt_ura, receipt_hash}` objects from
/// prior steps' metadata; they are encoded into the envelope's
/// `causal_context` (explicit `Empty` for a root step, `ReceiptRef`
/// scalar for one parent, ordered `ReceiptList` for a join).
/// `trace_id` is the mission run's id; it is stamped on the
/// envelope's operational-metadata `trace_id` field so the daemon
/// ledger groups every step of one run under one trace.
pub fn invoke_local_ability_with_invocation_meta(
    ability: &str,
    args: Value,
    subject: Option<String>,
    causal_parents: &[Value],
    step_timeout: Option<std::time::Duration>,
    trace_id: Option<&str>,
    callee_agent: Option<&str>,
) -> anyhow::Result<(Value, Value)> {
    crate::support::local_daemon_grpc::invoke_local_daemon_ability_with_invocation_meta(
        ability,
        args,
        subject,
        causal_parents,
        step_timeout,
        trace_id,
        callee_agent,
    )
}

/// Same as [`invoke_local_ability_with_invocation_meta`], but annotates the
/// returned metadata with the hosted agent whose local device signed the call.
///
/// This does NOT rewrite the hosted agent into Axon's caller. The signed
/// Invocation caller is the local daemon IPC system identity; hosted-agent
/// intent is carried as explicit delegation metadata and ability arguments,
/// not by rewriting caller identity.
pub fn invoke_local_ability_with_hosted_agent_delegation(
    ability: &str,
    args: Value,
    subject: Option<String>,
    causal_parents: &[Value],
    step_timeout: Option<std::time::Duration>,
    trace_id: Option<&str>,
    hosted_agent_ura: &str,
) -> anyhow::Result<(Value, Value)> {
    crate::support::local_daemon_grpc::invoke_local_daemon_ability_with_hosted_agent_delegation(
        ability,
        args,
        subject,
        causal_parents,
        step_timeout,
        trace_id,
        hosted_agent_ura,
    )
}

/// Standard error message for any CLI surface that semantically
/// requires the federation tier (cross-node enumeration, remote
/// dispatch, voice/video signaling). The federation Invoke surface
/// that would back these calls was removed by AXON-RFC-001 P1.5
/// and ships as a follow-up; until then, every command that
/// genuinely needs cross-node reach surfaces this exact message.
///
/// Centralised so:
///   * the wording stays byte-identical across surfaces (a script
///     can grep one substring),
///   * the operator sees one consistent name for the missing
///     subsystem instead of 8 variations of "federation gone",
///   * the day federation Invoke lands, deletion of this string
///     plus its callers is one PR rather than scavenger-hunt.
///
/// `action` is a short verb-phrase describing what the user was
/// trying to do (e.g. `"list remote devices"`, `"deploy ability to a
/// remote node"`); it is splice into the message so the operator
/// sees the verb that failed in front of the same explanation.
#[cfg(not(feature = "axon-pb"))]
pub fn federation_not_wired_error(action: &str) -> anyhow::Error {
    anyhow::Error::new(LocalInvokeFailure::DaemonOffline(format!(
        "{action} requires the `axon-pb` feature; rebuild with \
         `cargo build --features axon-pb` (production builds always do)."
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn local_ability_target_preserves_agent_owner_as_callee_and_subject() {
        let selector =
            crate::ura::AbilitySelector::parse("easynet:///r/acme/ability/alice.claude.weather")
                .expect("agent ability selector");
        let target = LocalAbilityTarget::from_selector(&selector);

        assert_eq!(target.dispatch_name(), "claude.weather");
        assert_eq!(target.callee_ura(), "easynet:///r/acme/agent/alice.claude");
        assert_eq!(
            target.default_subject_ura(),
            "easynet:///r/acme/agent/alice.claude"
        );
    }

    #[test]
    fn local_ability_target_uses_ability_subject_for_hub_owner() {
        let selector =
            crate::ura::AbilitySelector::parse("easynet:///r/acme/ability/hub.federation.resolve")
                .expect("hub ability selector");
        let target = LocalAbilityTarget::from_selector(&selector);

        assert_eq!(target.dispatch_name(), "federation.resolve");
        assert_eq!(target.callee_ura(), "easynet:///r/acme/hub");
        assert_eq!(
            target.default_subject_ura(),
            "easynet:///r/acme/ability/hub.federation.resolve"
        );
    }

    #[test]
    fn invoke_local_ability_surfaces_daemon_down_with_actionable_message() {
        // Fresh HOME: no Axon daemon socket can be accepting. The
        // compatibility helper must surface the same actionable
        // daemon-down message while routing through daemon.sock,
        // not the legacy control socket frame.
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let err =
            invoke_local_ability("observe.health", json!({})).expect_err("daemon-down must fail");
        let msg = format!("{err}");
        assert!(
            msg.contains("daemon not running"),
            "must say `daemon not running`; got: {msg}"
        );
        assert!(
            msg.contains("easynet runtime start"),
            "must point at `easynet [runtime] start`; got: {msg}"
        );
    }
}
