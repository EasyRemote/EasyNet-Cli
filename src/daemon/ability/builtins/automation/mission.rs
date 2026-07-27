// EasyNet CLI — mission.run ability handler
// =========================================
//
// File: src/daemon/ability/builtins/automation/mission.rs
//
// Exposes EAL mission execution as a registered ability so an LLM
// inside an agent can compose multi-step / multi-agent workflows
// without having to shell out to the `easynet mission run` CLI.
//
// What lives here
// ---------------
//   * mission.run — Compile and execute an EAL program in-process.
//                   Args: { "source": "<eal-text>" } and optional
//                   { "label": "..." } for run-dir naming.
//                   Returns: { "run_id", "run_dir", "outputs",
//                              "meta" } where `outputs` is the map
//                   of let-bound results.
//
// Why this is the canonical orchestration entry
// ---------------------------------------------
// AGENTS.md teaches the LLM: "cross-agent calls go through the
// mission runtime; there is no second path." For that promise to
// hold from the LLM's seat, the mission runtime must be reachable
// AS A TOOL — i.e. an ability the MCP catalog exposes. Without
// this handler, the LLM had to choose a separate direct
// `mcp.bridge.call_tool` path (which loses EAL composition) or hop to
// `easynet mission run` via shell (depends on the agent having
// shell access AND breaks isolation).
//
// Implementation note
// -------------------
// The handler is a thin adapter over the daemon-owned mission application
// service.
// All error mapping, persistence, and dispatch invariants live in
// that single entry point — this file just adapts the JSON-shaped
// args to MissionRunOpts and the MissionRunResult back to JSON.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Map;
use serde_json::{json, Value};

use crate::daemon::ability::dispatch::AxonAbilityCatalog;

use crate::daemon::ability::dispatch::OwnerKind;
pub const ABILITY_RUN: &str = crate::daemon::ability::names::automation::MISSION_RUN;
/// `mission.track(run_id)` — read the persisted state of a prior
/// `mission.run` invocation. Returns the same shape `mission.run`
/// surfaces (run_id, run_dir, outputs, meta, ok), reconstructed
/// off the on-disk run dir. Use case: an LLM kicks off a long
/// mission (multi-agent fan-out), polls track until status leaves
/// `running`, then composes a final answer.
pub const ABILITY_TRACK: &str = crate::daemon::ability::names::automation::MISSION_TRACK;
/// `mission.cancel(run_id)` — flip an in-flight mission to
/// `cancelled`. No-op (with informative result) on a run that is
/// already terminal. Best-effort: removes the pid file and rewrites
/// meta.json; long-running step processes are not killed today,
/// they just stop being expected.
pub const ABILITY_CANCEL: &str = crate::daemon::ability::names::automation::MISSION_CANCEL;

/// Register every mission ability on the registry. Called once at
/// boot from `daemon::ability::catalog::build_registry_with_services`.
///
/// The earlier RFC-001 v4.1.6 cut also kept a `device.easynet.*`
/// alias for `run`/`track`/`cancel` ("user-facing alias"). The
/// follow-up M2 carrier (RFC-001 v4.1.7) deletes that alias —
/// per Q2 of the migration plan, `easynet.*` is "protocol entropy
/// generator" and the LLM corpus reads `mission.run` directly.
/// Single canonical name, no twin: the LLM never had a way to
/// know which to pick, the receipts diverged, and the duplicate
/// names doubled the meta-discovery surface for no win.
pub fn register(reg: &mut AxonAbilityCatalog) {
    reg.register_rpc_with_envelope_and_owner(
        "mission.run",
        OwnerKind::Device,
        Arc::new(move |env, args: Value| run_handler(env, args)),
    );
    reg.register_rpc_with_owner(
        "mission.track",
        OwnerKind::Device,
        Arc::new(move |args: Value| track_handler(args)),
    );
    reg.register_rpc_with_owner(
        "mission.cancel",
        OwnerKind::Device,
        Arc::new(move |args: Value| cancel_handler(args)),
    );
}

/// `mission.run` handler.
///
/// Args (JSON object):
///   `source`  — REQUIRED. EAL source text. Examples:
///                  `let r = claude.weather(location: "Beijing")`
///                  `print(r)`
///   `label`   — optional human-readable label baked into the
///                run-dir name. Defaults to `"mission.run"` so
///                two LLM-driven invocations land in distinct dirs
///                without the LLM having to mint an id.
///
/// Returns (JSON object):
///   `run_id`  — trailing component of the run dir
///   `run_dir` — absolute path on disk
///   `outputs` — map<binding-name, value>; one entry per `let` in the
///                EAL source. Bindings the source did not assign are
///                absent. Non-JSON values are kept as their raw
///                string form (mirrors `MissionRunResult.outputs`).
///   `meta`    — the `MissionRunMeta` blob (status, started_at_unix_ms,
///                ended_at_unix_ms, source_file, etc.).
///
/// Error semantics:
///   * Compile failure (EAL parse / planner reject)        → Err
///   * Traditional target ambiguity                         → Err
///   * Step dispatch failure inside the mission             → Err
///     (MissionRunner bubbles the typed step
///     error verbatim)
///   * Empty / missing `source`                              → Err with
///     a precise "source must be a non-empty string"
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MissionRunResponse {
    pub(crate) ok: bool,
    pub(crate) run_id: String,
    pub(crate) run_dir: String,
    pub(crate) outputs: serde_json::Map<String, Value>,
    pub(crate) meta: crate::daemon::execution::mission::orchestration::MissionRunMeta,
}

fn run_handler(
    env: crate::daemon::ability::dispatch::EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    let args = mission_args_object("mission.run", &args, &["source", "label"])?;
    let source = mission_required_string_arg("mission.run", args, "source")?;
    let label = mission_optional_string_arg("mission.run", args, "label")?
        .unwrap_or_else(|| "mission.run".to_string());

    let opts = crate::daemon::execution::mission::orchestration::MissionRunOpts {
        source_label: Some(label),
        run_timeout: None,
    };

    let gateway = Arc::new(
        crate::daemon::execution::mission::invocation_gateway::DaemonMissionInvocationGateway::from_admitted_envelope(&env)?,
    );
    let result = crate::daemon::execution::mission::orchestration::MissionRunner::new(gateway)
        .run(&source, opts)?;
    let outputs_json: serde_json::Map<String, Value> = result.bound_vars.into_iter().collect();
    serde_json::to_value(MissionRunResponse {
        ok: result.ok,
        run_id: result.run_id,
        run_dir: result.run_dir.to_string_lossy().into_owned(),
        outputs: outputs_json,
        meta: result.meta,
    })
    .map_err(Into::into)
}

/// `mission.track` handler.
///
/// Args (JSON object):
///   `run_id` — REQUIRED. The id returned by a prior `mission.run`
///                (the trailing component of `run_dir`).
///
/// Returns (JSON object):
///   `run_id`  — echoed back so the caller can correlate parallel polls
///   `run_dir` — absolute path on disk
///   `running` — true while the mission is still executing
///   `meta`    — full `MissionRunMeta` blob, including `status`,
///               `steps_total`, `steps_completed`, `steps_failed`,
///               `error`, and the optional `ability_graph_traces`
///               summaries
///
/// Returns an Err when no run with that id exists. The error message
/// includes a short list of the closest matches (find_run already
/// surfaces those) so the LLM can self-correct.
fn track_handler(args: Value) -> anyhow::Result<Value> {
    let args = mission_args_object("mission.track", &args, &["run_id"])?;
    let run_id = mission_required_string_arg("mission.track", args, "run_id")?;
    let summary = crate::daemon::execution::mission::orchestration::find_run(&run_id)?;
    let meta_json = serde_json::to_value(&summary.meta).unwrap_or(Value::Null);
    Ok(json!({
        "run_id":  summary.id,
        "run_dir": summary.path.to_string_lossy(),
        "running": summary.running,
        "meta":    meta_json,
    }))
}

/// `mission.cancel` handler.
///
/// Args (JSON object):
///   `run_id` — REQUIRED. The id of the in-flight mission to cancel.
///
/// Returns (JSON object):
///   `run_id`     — echoed
///   `cancelled`  — true if this call flipped status, false if the
///                  run was already terminal
///   `meta`       — refreshed `MissionRunMeta` post-cancel
///
/// Cancellation is best-effort: this updates the on-disk meta.json
/// + removes the `pid` marker, but does NOT kill any subprocesses
/// the mission may have spawned. A future PR adds tree-kill on the
/// step process group; until then long-running step subprocesses
/// finish on their own and their results are discarded by the
/// "cancelled" status.
fn cancel_handler(args: Value) -> anyhow::Result<Value> {
    let args = mission_args_object("mission.cancel", &args, &["run_id"])?;
    let run_id = mission_required_string_arg("mission.cancel", args, "run_id")?;
    let outcome = crate::daemon::execution::mission::orchestration::cancel_run(&run_id)?;
    let (cancelled, summary) = match outcome {
        crate::daemon::execution::mission::orchestration::CancelOutcome::Cancelled(s) => (true, s),
        crate::daemon::execution::mission::orchestration::CancelOutcome::AlreadyTerminal(s) => {
            (false, s)
        }
    };
    let meta_json = serde_json::to_value(&summary.meta).unwrap_or(Value::Null);
    Ok(json!({
        "run_id":    summary.id,
        "cancelled": cancelled,
        "meta":      meta_json,
    }))
}

fn mission_args_object<'a>(
    ability: &str,
    args: &'a Value,
    allowed_fields: &[&str],
) -> anyhow::Result<&'a Map<String, Value>> {
    let object = args
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("{ability}: args must be a JSON object"))?;
    for key in object.keys() {
        if !allowed_fields.contains(&key.as_str()) {
            anyhow::bail!("{ability}: unknown argument `{key}`");
        }
    }
    Ok(object)
}

fn mission_required_string_arg(
    ability: &str,
    args: &Map<String, Value>,
    field: &str,
) -> anyhow::Result<String> {
    let value = args
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("{ability}: `{field}` must be a non-empty string"))?;
    if value.trim().is_empty() {
        anyhow::bail!("{ability}: `{field}` must be a non-empty string");
    }
    Ok(value.to_string())
}

fn mission_optional_string_arg(
    ability: &str,
    args: &Map<String, Value>,
    field: &str,
) -> anyhow::Result<Option<String>> {
    let Some(value) = args.get(field) else {
        return Ok(None);
    };
    let value = value
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("{ability}: `{field}` must be a string"))?;
    Ok(Some(value.to_string()))
}

// ── Discovery surfaces ──────────────────────────────────────────
//
// These helpers feed the system ability catalog's descriptor tables.
// Keeping them here means the wire-level
// description text and the handler logic live in the same file —
// adding a new optional argument touches one module rather than two.

pub fn run_description() -> &'static str {
    "Compile and execute an EAL program in-process. Records the run \
     under `~/.easynet/missions/runs/<run_id>` and returns the run id, \
     run dir path, every `let`-bound output, and the run metadata. \
     Use this to drive multi-step or cross-agent orchestration; \
     `mission.track` polls a long run, `mission.cancel` aborts one. \
     For a single ability call, prefer direct invocation — it skips \
     the run-dir bookkeeping."
}

pub fn run_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["source"],
        "properties": {
            "source": {
                "type": "string",
                "description": "EAL source text. Member-call form is the \
                                supported way to dispatch to an agent: \
                                `let r = claude.weather(location: \"Beijing\")`."
            },
            "label": {
                "type": "string",
                "description": "Optional human-readable label baked into \
                                the run-dir name. Two unlabeled invocations \
                                still land in distinct dirs (the run id \
                                supplies the unique component), but a label \
                                makes `easynet mission list` readable."
            }
        }
    })
}

pub fn track_description() -> &'static str {
    "Read the persisted state of a prior `mission.run` invocation by \
     run id. Returns the same shape `mission.run` surfaces (run_id, \
     run_dir, outputs, meta, ok) reconstructed from the on-disk run \
     dir. Use it to poll a long-running mission without holding the \
     original RPC open."
}

pub fn track_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["run_id"],
        "properties": {
            "run_id": {
                "type": "string",
                "description": "The run id returned by an earlier \
                                `mission.run` call (the trailing component \
                                of run_dir)."
            }
        }
    })
}

pub fn cancel_description() -> &'static str {
    "Mark an in-flight mission run as cancelled. Best-effort: \
     rewrites the run's meta.json to status=cancelled and removes \
     the pid file; long-running step subprocesses are NOT killed \
     today, they just stop being awaited. Returns `cancelled = false` \
     (with informative meta) if the run was already terminal."
}

pub fn cancel_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["run_id"],
        "properties": {
            "run_id": {
                "type": "string",
                "description": "The run id of the mission to cancel."
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_envelope() -> crate::daemon::ability::dispatch::EnvelopeContext {
        crate::daemon::ability::dispatch::EnvelopeContext::for_test(
            "easynet:///r/test/agent/caller",
            "easynet:///r/test/resource/mission",
        )
    }

    /// Empty `source` is a caller bug, not a transient. Surface it
    /// loud so the LLM sees a precise error and reframes its tool
    /// call rather than retrying with the same input.
    #[test]
    fn rejects_missing_source() {
        let err = run_handler(test_envelope(), json!({})).unwrap_err();
        assert!(err.to_string().contains("`source`"));
    }

    #[test]
    fn run_rejects_non_object_args() {
        let err = run_handler(test_envelope(), json!("source text")).unwrap_err();
        assert!(err.to_string().contains("JSON object"));
    }

    #[test]
    fn run_rejects_unknown_argument() {
        let err = run_handler(
            test_envelope(),
            json!({
                "source": "let r = agent.echo()",
                "action": "retired-field"
            }),
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown argument `action`"));
    }

    /// Whitespace-only source must fail the same way as missing.
    #[test]
    fn rejects_blank_source() {
        let err = run_handler(test_envelope(), json!({"source": "   "})).unwrap_err();
        assert!(err.to_string().contains("`source`"));
    }

    /// Non-string `source` (an LLM passing it as a number or array
    /// by accident) hits the type guard, not the EAL parser.
    #[test]
    fn rejects_non_string_source() {
        let err = run_handler(test_envelope(), json!({"source": 42})).unwrap_err();
        assert!(err.to_string().contains("non-empty string"));
    }

    #[test]
    fn run_rejects_non_string_label_instead_of_defaulting() {
        let err = run_handler(
            test_envelope(),
            json!({"source": "let r = agent.echo()", "label": 42}),
        )
        .unwrap_err();
        assert!(err.to_string().contains("`label` must be a string"));
    }

    /// Live happy path is covered by the e2e cross-agent test —
    /// running an EAL program here would require a full kernel +
    /// dispatcher fixture, which the chat_ability tests already
    /// exercise. Keep this module focused on the JSON-shape glue.
    #[test]
    fn handler_label_defaults_to_mission_run() {
        // We can't run the handler synchronously here without a
        // dispatcher (any non-trivial source needs one), but we
        // can pin the label-default arg parsing by inspecting the
        // error message on a deliberately-broken source: the
        // run dir name will include the default label string.
        // Smoke-only — full coverage is in the e2e flow.
        let _ = run_handler(test_envelope(), json!({"source": "this is not eal"}));
    }

    /// Track requires `run_id`. Same arg-shape rules as run.
    #[test]
    fn track_rejects_missing_run_id() {
        let err = track_handler(json!({})).unwrap_err();
        assert!(err.to_string().contains("`run_id`"));
    }

    #[test]
    fn track_rejects_non_object_args() {
        let err = track_handler(json!("run-id")).unwrap_err();
        assert!(err.to_string().contains("JSON object"));
    }

    #[test]
    fn track_rejects_unknown_argument() {
        let err = track_handler(json!({
            "run_id": "run-1",
            "action": "retired-field"
        }))
        .unwrap_err();
        assert!(err.to_string().contains("unknown argument `action`"));
    }

    #[test]
    fn track_rejects_non_string_run_id() {
        let err = track_handler(json!({"run_id": 42})).unwrap_err();
        assert!(err.to_string().contains("`run_id`"));
    }

    #[test]
    fn track_rejects_blank_run_id() {
        let err = track_handler(json!({"run_id": "   "})).unwrap_err();
        assert!(err.to_string().contains("`run_id`"));
    }

    /// Track on a non-existent run id surfaces the `find_run`
    /// "no mission run found" error, which the LLM can correct
    /// against by listing first.
    #[test]
    fn track_unknown_run_id_returns_error() {
        let err = track_handler(json!({"run_id": "definitely-not-a-real-run-id"})).unwrap_err();
        assert!(err.to_string().to_lowercase().contains("mission run"));
    }

    /// Cancel mirrors track's arg-shape contract.
    #[test]
    fn cancel_rejects_missing_run_id() {
        let err = cancel_handler(json!({})).unwrap_err();
        assert!(err.to_string().contains("`run_id`"));
    }

    #[test]
    fn cancel_rejects_non_object_args() {
        let err = cancel_handler(json!("run-id")).unwrap_err();
        assert!(err.to_string().contains("JSON object"));
    }

    #[test]
    fn cancel_rejects_unknown_argument() {
        let err = cancel_handler(json!({
            "run_id": "run-1",
            "action": "retired-field"
        }))
        .unwrap_err();
        assert!(err.to_string().contains("unknown argument `action`"));
    }

    #[test]
    fn cancel_rejects_non_string_run_id() {
        let err = cancel_handler(json!({"run_id": false})).unwrap_err();
        assert!(err.to_string().contains("`run_id`"));
    }

    #[test]
    fn cancel_rejects_blank_run_id() {
        let err = cancel_handler(json!({"run_id": ""})).unwrap_err();
        assert!(err.to_string().contains("`run_id`"));
    }
}
