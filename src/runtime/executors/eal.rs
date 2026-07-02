// EasyNet CLI — EAL Ability Executor
// ====================================
//
// File: src/runtime/executors/eal.rs
// Description: Implementation backing `[exec] kind = "eal"` in an
//              ability manifest. Renders the embedded EAL `source`
//              against the call's `args` JSON via the shared
//              `{{ name }}` template engine, then hands the rendered
//              source to `mission_runs::run_mission_inproc` — the
//              single canonical EAL entry point that `easynet
//              mission run` and `easynet.run` already share.
//
// Why this executor exists at all
// -------------------------------
// Curator-published abilities should be able to compose existing
// abilities into reusable workflows without inventing a second
// orchestration surface. The two we considered:
//
//   1. A bespoke "ability_seq" mini-language embedded in the
//      manifest. Cheap to add, but the operator now has TWO surfaces
//      to learn (EAL + ability_seq) and the second one accumulates
//      ad-hoc features over time.
//   2. EAL itself, the surface the operator already uses with
//      `easynet.run` and `mission run`. One surface, one mental
//      model, one set of debugging tools (mission run dirs, traces,
//      bound_vars).
//
// The executor goes with (2). The cost is one extra crate boundary
// (this file → mission_runs::run_mission_inproc) and a soft cap on
// embedded source size; the win is the curator-authored workflow is
// indistinguishable, at execution time, from a hand-written `.eal`
// file the operator could run themselves.
//
// Result extraction
// -----------------
// EAL has no native "return value" — a mission's observable output
// is its `bound_vars` map (variables created by `let` bindings).
// Two cases:
//
//   * `result_binding = "<name>"` (recommended) — the executor
//     extracts `bound_vars["<name>"]` and surfaces it as the
//     envelope's `result` field. The author opts in to a specific
//     return shape; missing the binding is an error so a typo in the
//     manifest fails loud.
//   * `result_binding` absent — the entire `bound_vars` map is
//     surfaced as `result`. Useful for a workflow that creates
//     several artifacts the caller wants all of (e.g. a "publish
//     ability + emit descriptor" two-step).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use crate::cli::mission_runs::{run_mission_inproc, MissionRunOpts};
use crate::core::ability_spec::EalExec;
use crate::runtime::context::ParentInvocationContext;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// Default per-call timeout. An EAL ability can fan out across
/// agent member-calls that each go through `<agent>.chat`; with the
/// per-agent dispatch default at 1 hour, a few sequential
/// agent-calls inside one EAL ability can legitimately blow past
/// any sub-hour ceiling. Setting this to 1 hour keeps EAL exec in
/// the same "long task is normal" regime as the rest of the LLM-
/// facing surface. Manifests with a known short workflow should
/// pin `timeout_seconds` explicitly.
const DEFAULT_TIMEOUT_SECS: u64 = 3600;
pub const AXON_INVOCATION_CONTEXT_KEY: &str = "__axon_invocation";

/// Run an EAL ability. Returns a JSON envelope
/// `{"result": <bound_vars or single binding>, "fulfilled_by": "eal",
/// "run_id": "<mission run id>", "elapsed_ms": N, "ok": <bool>}`.
/// Errors come back as `Err(anyhow)`; the dispatcher surfaces them
/// as typed error frames the same way it does for shell + http.
pub fn run_eal_exec(
    spec: &EalExec,
    args: &Value,
    timeout: Option<Duration>,
) -> anyhow::Result<Value> {
    run_eal_exec_with_invocation_context(spec, args, None, timeout)
}

/// Run an EAL ability with daemon invocation context available to templates.
///
/// Plugin-owned EAL abilities receive the parent AXIOM envelope under the
/// reserved `__axon_invocation` argument key. This keeps caller, subject, nonce,
/// and causal context visible to composed EAL missions without asking plugin
/// authors to smuggle protocol state through business arguments.
pub fn run_eal_exec_with_invocation_context(
    spec: &EalExec,
    args: &Value,
    invocation_context: Option<Value>,
    timeout: Option<Duration>,
) -> anyhow::Result<Value> {
    let render_args = args_with_invocation_context(args, invocation_context.clone())?;
    let parent_invocation = invocation_context
        .map(ParentInvocationContext::from_json_value)
        .transpose()
        .map_err(|err| anyhow::anyhow!("eal executor: invalid invocation context: {err}"))?;
    // Render `{{ name }}` placeholders in the embedded source. We
    // render BEFORE handing to the EAL parser so a template error
    // (missing arg, unclosed brace, …) surfaces with the executor
    // label — not as a confusing parse error several layers down.
    let rendered = crate::runtime::executors::template::render_template(
        &spec.source,
        &render_args,
        "eal executor",
    )?;

    let started = Instant::now();
    let _ = timeout.unwrap_or_else(|| Duration::from_secs(DEFAULT_TIMEOUT_SECS));

    // Hand to the canonical EAL entry point. We deliberately do NOT
    // re-implement compile/dispatch here — that would create a second
    // EAL surface that drifts from `easynet.run` over time, defeating
    // the entire reason this executor exists (one surface, one
    // mental model). See module doc.
    //
    // `source_label` is set so the on-disk run dir's meta records
    // "ability:eal" instead of an empty label; an operator scanning
    // `~/.easynet/missions/runs/*/meta.json` can tell which runs
    // came from a curator-published EAL ability vs a hand-rolled
    // mission.
    let opts = MissionRunOpts {
        source_label: Some("ability:eal".to_string()),
        trace_path: None,
        invocation_context: parent_invocation,
    };
    let run = run_mission_inproc(&rendered, opts)
        .map_err(|e| anyhow::anyhow!("eal executor: mission run failed: {e}"))?;

    let elapsed_ms = started.elapsed().as_millis() as u64;

    // Extract the result value per the manifest's `result_binding`
    // contract. See module doc for the two cases.
    let result_value: Value = match &spec.result_binding {
        Some(binding) => run.bound_vars.get(binding).cloned().ok_or_else(|| {
            anyhow::anyhow!(
                "eal executor: manifest pinned result_binding=`{}` but the rendered \
                 mission produced no such binding (available: {:?})",
                binding,
                run.bound_vars.keys().collect::<Vec<_>>()
            )
        })?,
        None => {
            // No specific binding requested — surface the whole map
            // as a JSON object so the caller can pick what they
            // need. We rebuild rather than serialize the HashMap
            // directly to keep the key ordering deterministic for
            // tests.
            let mut map = serde_json::Map::new();
            let mut keys: Vec<&String> = run.bound_vars.keys().collect();
            keys.sort();
            for k in keys {
                if let Some(v) = run.bound_vars.get(k) {
                    map.insert(k.clone(), v.clone());
                }
            }
            Value::Object(map)
        }
    };

    Ok(json!({
        "result": result_value,
        "fulfilled_by": "eal",
        "run_id": run.run_id,
        "elapsed_ms": elapsed_ms,
        "ok": run.ok,
    }))
}

fn args_with_invocation_context(
    args: &Value,
    invocation_context: Option<Value>,
) -> anyhow::Result<Value> {
    let Some(invocation_context) = invocation_context else {
        return Ok(args.clone());
    };
    let mut object = match args {
        Value::Object(map) => map.clone(),
        other => {
            return Ok(json!({
                "args": other,
                AXON_INVOCATION_CONTEXT_KEY: invocation_context,
            }));
        }
    };
    if object.contains_key(AXON_INVOCATION_CONTEXT_KEY) {
        anyhow::bail!(
            "eal executor: `{AXON_INVOCATION_CONTEXT_KEY}` is reserved for daemon invocation context"
        );
    }
    object.insert(AXON_INVOCATION_CONTEXT_KEY.to_string(), invocation_context);
    Ok(Value::Object(object))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::test_support::HomeGuard;

    /// Template substitution must run BEFORE the EAL parser AND
    /// before `run_mission_inproc` reaches `runtime::config::load`.
    /// A missing arg error attributable to the executor's caller
    /// label — not to a parser or daemon-state error several layers
    /// down — is the contract this test pins. It fires entirely on
    /// the in-process render path, so it does NOT depend on a live
    /// daemon and is the only `eal_executor` unit test that runs
    /// here. End-to-end coverage of the mission-run path
    /// (`run_eal_exec_with_empty_mission`,
    /// `run_eal_exec_renders_template_before_parse`,
    /// `run_eal_exec_errors_on_missing_result_binding`) needs a
    /// running daemon and lives in the Phase 7 integration suite
    /// where a daemon fixture is available.
    #[test]
    fn run_eal_exec_errors_on_missing_template_arg() {
        let _g = HomeGuard::new();
        let spec = EalExec {
            source: "mission \"{{ name }}\" {}".to_string(),
            result_binding: None,
        };
        let err = run_eal_exec(&spec, &json!({}), None).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("eal executor"), "label missing: {msg}");
        assert!(msg.contains("name"), "missing key not named: {msg}");
    }

    #[test]
    fn invocation_context_is_available_to_plugin_eal_templates() {
        let args = args_with_invocation_context(
            &json!({}),
            Some(json!({"subject": "easynet:///r/acme/resource/test"})),
        )
        .expect("context args");
        assert_eq!(
            args[AXON_INVOCATION_CONTEXT_KEY]["subject"],
            "easynet:///r/acme/resource/test"
        );
    }

    #[test]
    fn user_args_cannot_shadow_invocation_context() {
        let spec = EalExec {
            source: "mission \"x\" {}".to_string(),
            result_binding: None,
        };
        let err = run_eal_exec_with_invocation_context(
            &spec,
            &json!({"__axon_invocation": {}}),
            Some(json!({"subject": "s"})),
            None,
        )
        .unwrap_err();
        assert!(format!("{err}").contains("reserved"));
    }
}
