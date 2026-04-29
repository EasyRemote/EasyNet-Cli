// EasyNet CLI — Local ability invocation helper
// ==============================================
//
// File: src/support/local_invoke.rs
// Description: One function — `invoke_local_ability(name, args)` —
//              that every CLI subcommand uses to dispatch to the
//              local daemon's AbilityDispatcher via the Control
//              plane (~/.easynet/control.sock).
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

use anyhow::{bail, Context};
use serde_json::Value;

use crate::services::control::discovery;
use crate::services::control::frames::{IncomingFrame, OutgoingFrame};

/// Invoke an ability against the local daemon's dispatcher.
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
/// Errors raised here:
///   * daemon-not-running: control.json missing → "daemon not running"
///   * IPC transport: connect/round_trip failure → bubbled with context
///   * Daemon-side error frame: rendered as
///     `daemon error invoking '<ability>' (code=<c>): <msg>`
///   * Unexpected wire frame: bubbled verbatim so a regression in the
///     IPC layer surfaces visibly
pub fn invoke_local_ability(ability: &str, args: Value) -> anyhow::Result<Value> {
    let control_json = discovery::default_path();
    if !control_json.exists() {
        bail!(
            "daemon not running (no control.json at {}). \
             Start it with `easynet runtime start`.",
            control_json.display()
        );
    }

    // Same single-thread tokio runtime pattern every other sync
    // CLI subcommand uses — clap's command dispatch is sync, and
    // the IPC client is async (tokio UDS).
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime")?;

    let ability_owned = ability.to_string();
    runtime.block_on(async move {
        let mut client = crate::ffi::client::connect(&control_json)
            .await
            .context("connect to local daemon control socket")?;
        let request_id = format!("cli-{}", short_correlation_id());
        let resp = client
            .round_trip(IncomingFrame::Invoke {
                request_id: request_id.clone(),
                ability: ability_owned.clone(),
                args,
            })
            .await
            .with_context(|| format!("invoke '{ability_owned}' via local daemon"))?;
        match resp {
            OutgoingFrame::Result {
                request_id: rid,
                value,
                ..
            } => {
                if rid != request_id {
                    bail!(
                        "daemon Result returned request_id {rid:?} but we sent {request_id:?}"
                    );
                }
                Ok(value)
            }
            OutgoingFrame::Error {
                request_id: rid,
                code,
                message,
                ..
            } => bail!(
                "daemon error invoking '{ability_owned}' (request_id={:?}, code={code}): {message}",
                rid.unwrap_or_default()
            ),
            other => bail!(
                "daemon returned an unexpected frame for an Invoke request: {other:?}"
            ),
        }
    })
}

/// Non-cryptographic correlation id for the request_id field. Same
/// shape every CLI surface uses; pulled into one helper so the
/// "ns since epoch, hex" choice is one line to evolve.
fn short_correlation_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}")
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
pub fn federation_not_wired_error(action: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "{action} requires the federation Invoke surface, which was removed by \
         AXON-RFC-001 P1.5 and has not yet been re-published as a federation-tier \
         ability. Local-only operations remain available — see `easynet ability list` \
         for what this node can do without federation. The replacement (Invoke \
         against an Agent ability on the realm) ships in a follow-up; this command \
         will be re-wired without changing its CLI shape when it lands."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn invoke_local_ability_surfaces_daemon_down_with_actionable_message() {
        // We can't easily start a daemon in unit-test scope, but the
        // `daemon not running` branch is reachable when control.json
        // is missing. Use a HomeGuard'd fresh HOME to guarantee
        // absence; the message MUST tell the operator how to recover
        // (`easynet runtime start`).
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let err = invoke_local_ability("observe.health", json!({}))
            .expect_err("daemon-down must fail");
        let msg = format!("{err}");
        assert!(
            msg.contains("daemon not running"),
            "must say `daemon not running`; got: {msg}"
        );
        assert!(
            msg.contains("easynet runtime start"),
            "must point at `easynet runtime start`; got: {msg}"
        );
    }
}
