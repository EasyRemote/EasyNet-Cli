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
) -> anyhow::Result<(Value, Value)> {
    crate::support::local_daemon_grpc::invoke_local_daemon_ability_with_invocation_meta(
        ability,
        args,
        subject,
        causal_parents,
        step_timeout,
        trace_id,
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
    anyhow::anyhow!(
        "{action} requires the `axon-pb` feature; rebuild with \
         `cargo build --features axon-pb` (production builds always do)."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
