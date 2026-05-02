// EasyNet CLI — a2a.client.send_task ability handler (C-M10-iii)
// =================================================================
//
// File: src/runtime/agents/a2a_client_ability.rs
//
// Outbound A2A: lets a local caller dispatch an Invoke against a
// remote node's ability, surfaced as a first-class ability
// (`a2a.client.send_task`) so the caller doesn't reach into
// AbilityDispatcher directly. Pairs with `a2a.bridge.send_task`
// (the inbound side) — both surfaces ride the same Invoke
// pipeline; the difference is which direction crosses the wire.
//
// Why an ability and not a CLI subcommand
// ---------------------------------------
// Anything an interactive operator can do, an in-process planner
// (or a hosted Agent) should also be able to do. Naming the
// outbound surface as an ability means a future LLM-driven
// orchestrator can compose `meta.list_abilities` → pick a target
// → `a2a.client.send_task` to a remote node — same call shape as
// dispatching against a local ability, no special-case planner
// glue.
//
// Why this ISN'T `send_a2a_task`
// ------------------------------
// AXON-RFC-001 P1.5 deleted the underlying `send_a2a_task` axon
// helper (it now bails with a deprecation message). The new
// canonical path is "use Invoke against the appropriate Agent
// ability." This ability is the wrapper that does exactly that:
// builds an InvocationTarget with `TargetScope::Remote{node}` and
// hands it to the dispatcher; the dispatcher's existing remote
// path routes through the GatewayApi.
//
// What lives here
// ---------------
//   * a2a.client.send_task — { target_node_uri, agent_name,
//                              skill_name, args }. Resolves to
//                              ability `<agent_name>.<skill_name>`
//                              on the named remote node.
//
// What does NOT live here yet
// ---------------------------
//   * Streaming / bidi outbound — same handler shape, different
//     dispatcher entry (execute_stream / execute_bidi). Land
//     when an actual remote streaming caller surfaces; the unary
//     surface covers every concrete request known today.
//   * a2a.client.list — outbound discovery. The realm hub's
//     `federation.subscribe_directory` (C-M11) is the right
//     surface for "what nodes can I talk to"; this ability
//     focuses on the dispatch primitive.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::runtime::ability_dispatch::{AbilityDispatcher, LocalAbilityRegistry};
use crate::runtime::domain::NodeId;
use crate::runtime::invocation_target::{CallMode, InvocationTarget, TargetScope};

pub const ABILITY_SEND_TASK: &str = "a2a.client.send_task";

/// Process-wide dispatcher handle. Populated by the daemon bin
/// after `AbilityDispatcher::new(...)` completes; left unset in
/// tests, where the handler's not-initialised path is what we
/// verify. Static rather than per-`register` arg because the
/// dispatcher is built downstream of `build_registry_with_services`,
/// so threading a per-call OnceLock through every caller would
/// touch every test that builds a registry. The static seam
/// isolates the concern to the daemon bin's boot sequence.
///
/// Tests that want the populated path live as integration tests at
/// the daemon level (where `set_dispatcher` runs after the boot
/// sequence completes). Reaching the populated path from a unit
/// test would race other tests through this same static OnceLock.
static DISPATCHER_HANDLE: std::sync::OnceLock<Arc<AbilityDispatcher>> = std::sync::OnceLock::new();

/// Daemon bin's post-boot hook to wire the dispatcher into the
/// outbound-A2A handler. Idempotent (subsequent calls no-op since
/// OnceLock only takes the first set), so a daemon that re-runs its
/// boot sequence (hot-reload future) doesn't crash here.
pub fn set_dispatcher(dispatcher: Arc<AbilityDispatcher>) {
    let _ = DISPATCHER_HANDLE.set(dispatcher);
}

/// Register a2a.client.send_task on the registry. Closes over the
/// process-wide DISPATCHER_HANDLE — populated separately by
/// `set_dispatcher` after the dispatcher exists. Until that lock
/// is set, the handler returns ok:false on every call.
pub fn register(reg: &mut LocalAbilityRegistry) {
    reg.register_rpc(
        ABILITY_SEND_TASK,
        Arc::new(move |args: Value| send_task_handler(args)),
    );
}

/// `a2a.client.send_task` handler.
///
/// Args: `{ "target_node_uri": "<URI>", "agent_name": "<agent>",
///          "skill_name": "<verb>", "args": <json-value> }`.
///
/// `target_node_uri` is the remote node's identifier as the
/// resolver / gateway expects it — for v1 a `NodeId` is just a
/// string wrapper, so we pass the caller's value through.
///
/// Returns: `{ ok, result?, error? }` — same envelope shape
/// `a2a.bridge.send_task` (the inbound side) returns, so a planner
/// that handles the inbound shape handles the outbound shape too.
///
/// Failure paths surface as `{ok:false, error:...}` rather than
/// `Err`. The dispatcher's error from a remote bail-out (e.g. the
/// gateway can't reach the node) becomes a structured caller-
/// visible response; only programmer errors (lock poisoning,
/// genuinely impossible states) bubble as `Err`.
fn send_task_handler(args: Value) -> anyhow::Result<Value> {
    let target_node = match required_nonempty_string(&args, "target_node_uri") {
        Ok(s) => s,
        Err(msg) => return Ok(error_response(&msg)),
    };
    let agent_name = match required_nonempty_string(&args, "agent_name") {
        Ok(s) => s,
        Err(msg) => return Ok(error_response(&msg)),
    };
    let skill_name = match required_nonempty_string(&args, "skill_name") {
        Ok(s) => s,
        Err(msg) => return Ok(error_response(&msg)),
    };
    let task_args = args.get("args").cloned().unwrap_or(Value::Null);

    let Some(dispatcher) = DISPATCHER_HANDLE.get() else {
        return Ok(error_response(
            "dispatcher not initialised (production: daemon-bin's set_dispatcher hook; \
             tests deliberately leave this unset)",
        ));
    };

    let target = InvocationTarget {
        scope: TargetScope::Remote {
            node: NodeId::new(target_node),
        },
        ability: format!("{agent_name}.{skill_name}"),
        normalized_args: task_args,
        call_mode: CallMode::Rpc,
        // PR-DISPATCHER-SUBJECT: A2A skill invocation does not
        // currently carry an AXIOM `subject`; the remote skill is
        // identified by ability name only. Future A2A wire schema
        // extension can populate this.
        subject: None,
    };

    // The dispatcher's error already names the failure (e.g. "no
    // gateway", "remote node unreachable"); we forward verbatim
    // rather than prefix "remote dispatch failed:" — that prefix
    // would double up in the rendered message.
    match dispatcher.execute_rpc(target) {
        Ok(value) => Ok(json!({ "ok": true, "result": value })),
        Err(e) => Ok(error_response(&format!("{e}"))),
    }
}

/// Pull a required, non-empty string field out of `args`. Returns
/// the string on success; returns the caller-visible error message
/// on absence/wrong-type/empty so the call site can wrap it in an
/// error_response without a separate format!.
fn required_nonempty_string(args: &Value, key: &str) -> Result<String, String> {
    match args.get(key).and_then(Value::as_str) {
        Some(s) if !s.is_empty() => Ok(s.to_string()),
        _ => Err(format!(
            "`{key}` is required and must be a non-empty string"
        )),
    }
}

fn error_response(message: &str) -> Value {
    json!({ "ok": false, "error": message })
}

// ── Discovery surfaces ────────────────────────────────────────

pub fn send_task_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["target_node_uri", "agent_name", "skill_name"],
        "properties": {
            "target_node_uri": {"type": "string", "minLength": 1},
            "agent_name": {"type": "string", "minLength": 1},
            "skill_name": {"type": "string", "minLength": 1},
            "args": {
                "description": "Free-form per-skill args; shape per the remote skill's input_schema."
            },
        },
        "additionalProperties": false,
    })
}

pub fn send_task_description() -> &'static str {
    "Outbound A2A: dispatch an RPC Invoke against a remote node's \
     ability. Resolves to `<agent_name>.<skill_name>` on the named \
     node and routes through the GatewayApi. Returns {ok:true,result} \
     on success; remote failures and dispatcher errors surface as \
     {ok:false,error} so callers can branch without a try/catch."
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests deliberately leave the process-wide DISPATCHER_HANDLE
    /// unset. The handler's unset-path returns ok:false on every
    /// call, which is what we test here. The populated path is
    /// exercised by the daemon's boot integration test (where
    /// `set_dispatcher` runs after `AbilityDispatcher::new`) — doing
    /// it here would race other tests through the static OnceLock.
    fn fresh_registry() -> Arc<LocalAbilityRegistry> {
        let mut reg = LocalAbilityRegistry::new();
        register(&mut reg);
        Arc::new(reg)
    }

    #[test]
    fn registration_makes_send_task_dispatchable() {
        let arc = fresh_registry();
        assert!(arc.get_rpc(ABILITY_SEND_TASK).is_some());
    }

    #[test]
    fn send_task_missing_target_node_uri_returns_ok_false() {
        let arc = fresh_registry();
        let handler = arc.get_rpc(ABILITY_SEND_TASK).unwrap();
        let resp = handler(json!({
            "agent_name": "claude",
            "skill_name": "chat",
        }))
        .unwrap();
        assert_eq!(resp["ok"], false);
        let err = resp["error"].as_str().unwrap();
        assert!(
            err.contains("`target_node_uri`"),
            "error must name the missing field; got {err:?}"
        );
    }

    #[test]
    fn send_task_missing_agent_name_returns_ok_false() {
        let arc = fresh_registry();
        let handler = arc.get_rpc(ABILITY_SEND_TASK).unwrap();
        let resp = handler(json!({
            "target_node_uri": "easynet:///r/acme/node/N1",
            "skill_name": "chat",
        }))
        .unwrap();
        assert_eq!(resp["ok"], false);
        assert!(resp["error"].as_str().unwrap().contains("`agent_name`"));
    }

    #[test]
    fn send_task_missing_skill_name_returns_ok_false() {
        let arc = fresh_registry();
        let handler = arc.get_rpc(ABILITY_SEND_TASK).unwrap();
        let resp = handler(json!({
            "target_node_uri": "easynet:///r/acme/node/N1",
            "agent_name": "claude",
        }))
        .unwrap();
        assert_eq!(resp["ok"], false);
        assert!(resp["error"].as_str().unwrap().contains("`skill_name`"));
    }

    #[test]
    fn send_task_empty_string_field_returns_ok_false() {
        let arc = fresh_registry();
        let handler = arc.get_rpc(ABILITY_SEND_TASK).unwrap();
        let resp = handler(json!({
            "target_node_uri": "",
            "agent_name": "claude",
            "skill_name": "chat",
        }))
        .unwrap();
        assert_eq!(resp["ok"], false);
    }

    #[test]
    fn send_task_with_unset_dispatcher_handle_returns_ok_false_no_panic() {
        // The DISPATCHER_HANDLE is never populated by tests (see
        // module-doc on the static). All-fields-valid call MUST
        // surface the not-initialised error, NOT panic.
        let arc = fresh_registry();
        let handler = arc.get_rpc(ABILITY_SEND_TASK).unwrap();
        let resp = handler(json!({
            "target_node_uri": "easynet:///r/acme/node/N1",
            "agent_name": "claude",
            "skill_name": "chat",
        }))
        .unwrap();
        assert_eq!(resp["ok"], false);
        assert!(resp["error"].as_str().unwrap().contains("not initialised"));
    }

    #[test]
    fn send_task_input_schema_requires_three_string_fields() {
        let s = send_task_input_schema();
        let req = s["required"].as_array().unwrap();
        for field in ["target_node_uri", "agent_name", "skill_name"] {
            assert!(
                req.iter().any(|v| v == field),
                "required field {field} missing from schema"
            );
            assert_eq!(s["properties"][field]["minLength"], 1);
        }
        assert_eq!(s["additionalProperties"], false);
    }
}
