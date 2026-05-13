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
            // Convert "connect refused / no such file" failures into the
            // same actionable "daemon not running" message the pre-check
            // above raises when control.json is absent. The pre-check
            // catches a clean uninstall; this catches the *stale* case
            // where control.json is left over but the daemon process
            // died (e.g. machine restart, `kill -9`, or a `start` that
            // crashed without cleaning up). Both states present
            // identically to the user — they just need to run
            // `easynet start`.
            .map_err(|e| friendlify_connect_error(e, &control_json))?;
        let request_id = format!("cli-{}", short_correlation_id());
        let resp = client
            .round_trip(IncomingFrame::Invoke {
                request_id: request_id.clone(),
                ability: ability_owned.clone(),
                args,
                subject,
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
                    bail!("daemon Result returned request_id {rid:?} but we sent {request_id:?}");
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
            other => bail!("daemon returned an unexpected frame for an Invoke request: {other:?}"),
        }
    })
}

/// Translate an FFI client `connect` failure into a user-facing
/// "daemon not running" message when the OS-level error is one of
/// the well-known "no listener at this socket" kinds. Anything
/// else bubbles up verbatim — we don't want to mask a genuine
/// permission / path / IPC bug behind the friendly message.
///
/// Three OS errors map to "daemon down":
///   * `ConnectionRefused` (errno 61 / ECONNREFUSED) — control.sock
///     exists but no process is listen()-ing on it. This is what
///     silan hit: previous `easynet start` left `control.sock` on
///     disk; the daemon process died; new `connect()` is refused.
///   * `NotFound` — control.sock has been unlinked since we did
///     the `control.json` existence pre-check.
///   * `AddrNotAvailable` — Linux variant of the same race.
fn friendlify_connect_error(err: anyhow::Error, control_json: &std::path::Path) -> anyhow::Error {
    let chain_has_daemon_down = err.chain().any(|cause| {
        if let Some(io_err) = cause.downcast_ref::<std::io::Error>() {
            matches!(
                io_err.kind(),
                std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::NotFound
                    | std::io::ErrorKind::AddrNotAvailable,
            )
        } else {
            false
        }
    });
    if chain_has_daemon_down {
        anyhow::anyhow!(
            "daemon not running (control socket at {} is not accepting \
             connections — its process likely died or was killed without \
             cleaning up). Start it with `easynet start`.",
            control_json.parent().unwrap_or(control_json).display()
        )
    } else {
        err.context("connect to local daemon control socket")
    }
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
        // Branch 1 — control.json missing entirely (clean install,
        // never ran `start`). HomeGuard gives us a fresh empty HOME
        // so `discovery::default_path()` resolves to a path that
        // does not exist. The pre-check at the top of
        // invoke_local_ability_with_subject must catch this and
        // emit an actionable message that names the recovery
        // command verbatim.
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let err =
            invoke_local_ability("observe.health", json!({})).expect_err("daemon-down must fail");
        let msg = format!("{err}");
        assert!(
            msg.contains("daemon not running"),
            "must say `daemon not running`; got: {msg}"
        );
        assert!(
            msg.contains("easynet runtime start") || msg.contains("easynet start"),
            "must point at `easynet [runtime] start`; got: {msg}"
        );
    }

    #[test]
    fn friendlify_connect_error_translates_econnrefused_to_daemon_not_running() {
        // Branch 2 — control.json exists but the daemon process is
        // gone (crashed / killed without unlinking control.sock).
        // The connect() call fails with ECONNREFUSED. The pre-check
        // wouldn't fire here because the file is on disk.
        // friendlify_connect_error must turn the io error into the
        // same actionable message users get from Branch 1.
        let io_err = std::io::Error::from(std::io::ErrorKind::ConnectionRefused);
        let wrapped = anyhow::Error::new(io_err).context("FFI client: connect to /tmp/sock failed");
        let friendly = friendlify_connect_error(wrapped, std::path::Path::new("/tmp/control.json"));
        let msg = format!("{friendly}");
        assert!(
            msg.contains("daemon not running"),
            "must say `daemon not running`; got: {msg}"
        );
        assert!(
            msg.contains("easynet start"),
            "must point at `easynet start`; got: {msg}"
        );
    }

    #[test]
    fn friendlify_connect_error_passes_through_other_errors() {
        // A genuine bug (e.g. permission denied on the socket dir)
        // must surface as-is — masking it as "daemon not running"
        // would send the operator chasing the wrong fix.
        let io_err = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let wrapped = anyhow::Error::new(io_err).context("FFI client: connect to /tmp/sock failed");
        let friendly = friendlify_connect_error(wrapped, std::path::Path::new("/tmp/control.json"));
        let msg = format!("{friendly}");
        assert!(
            !msg.contains("daemon not running"),
            "permission errors must NOT be rewritten to daemon-down; got: {msg}"
        );
        assert!(
            msg.contains("connect to local daemon control socket"),
            "must keep the original context line; got: {msg}"
        );
    }
}
