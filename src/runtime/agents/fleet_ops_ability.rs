// EasyNet CLI — fleet.* operations abilities
// ==========================================
//
// File: src/runtime/agents/fleet_ops_ability.rs
// Description: The seven `fleet.*` abilities the CLI's
//              device/ability subcommands invoke. Replaces the
//              former direct calls to bridge fns
//              (`list_nodes`, `publish_capability`, etc.) that
//              AXON-RFC-001 P1.5 removed; the ability surface
//              survives unchanged regardless of which transport
//              backs them, in line with the ontology that says
//              "every action is an ability invocation."
//
// Abilities registered here
// -------------------------
//   fleet.list_nodes        List device nodes (this device + known peers).
//   fleet.describe_node     Describe one node by id.
//   fleet.remove_node       Remove a node from the fleet (federation-tier).
//   fleet.deploy_ability    Publish an ability bundle to a target node.
//   fleet.uninstall_ability Uninstall a previously deployed ability.
//   fleet.exec_remote       One-shot command execution on a target node.
//   fleet.register_self     Register THIS device with the realm (lifecycle).
//   fleet.deregister_self   Inverse of register_self at shutdown.
//
// Routing model
// -------------
// Every handler accepts `node_id` (or `target_node_id`); the value
// `"local"` (or absent) means "this device" and is fully implemented
// in-process. Any other id is a federation-tier target — the
// transport that fans the call out across the realm was removed by
// AXON-RFC-001 P1.5 and will be re-wired as a federation Invoke
// surface. Until then, those handlers return a typed
// `federation_not_wired` error so callers see the same actionable
// message every CLI surface produces.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::runtime::ability_dispatch::LocalAbilityRegistry;
use crate::runtime::agents::federation_probe;

pub const ABILITY_LIST_NODES: &str = "fleet.list_nodes";
pub const ABILITY_DESCRIBE_NODE: &str = "fleet.describe_node";
pub const ABILITY_REMOVE_NODE: &str = "fleet.remove_node";
pub const ABILITY_DEPLOY_ABILITY: &str = "fleet.deploy_ability";
pub const ABILITY_UNINSTALL_ABILITY: &str = "fleet.uninstall_ability";
pub const ABILITY_EXEC_REMOTE: &str = "fleet.exec_remote";
pub const ABILITY_REGISTER_SELF: &str = "fleet.register_self";
pub const ABILITY_DEREGISTER_SELF: &str = "fleet.deregister_self";

/// Register every fleet.* operation handler on `reg`. Called once
/// at daemon boot from `runtime::agents::build_registry_with_services`.
pub fn register(reg: &mut LocalAbilityRegistry) {
    reg.register_rpc(
        ABILITY_LIST_NODES,
        Arc::new(|args| list_nodes_handler(args)),
    );
    reg.register_rpc(
        ABILITY_DESCRIBE_NODE,
        Arc::new(|args| describe_node_handler(args)),
    );
    reg.register_rpc(
        ABILITY_REMOVE_NODE,
        Arc::new(|args| remove_node_handler(args)),
    );
    reg.register_rpc(
        ABILITY_DEPLOY_ABILITY,
        Arc::new(|args| deploy_ability_handler(args)),
    );
    reg.register_rpc(
        ABILITY_UNINSTALL_ABILITY,
        Arc::new(|args| uninstall_ability_handler(args)),
    );
    reg.register_rpc(
        ABILITY_EXEC_REMOTE,
        Arc::new(|args| exec_remote_handler(args)),
    );
    reg.register_rpc(
        ABILITY_REGISTER_SELF,
        Arc::new(|args| register_self_handler(args)),
    );
    reg.register_rpc(
        ABILITY_DEREGISTER_SELF,
        Arc::new(|args| deregister_self_handler(args)),
    );
}

// ── Helpers ──────────────────────────────────────────────────────

/// Resolve the local node's identity from credentials + runtime state.
/// Returns the `(node_id, tenant_id, hub_endpoint, paired)` tuple that
/// every fleet handler needs to know "what is this device". `paired
/// = false` when `~/.easynet/credentials.json` is absent — the
/// daemon may still serve local abilities, but federation-tier
/// answers should reflect the unpaired state.
fn local_identity() -> (String, String, Option<String>, bool) {
    let local = federation_probe::local_identity();
    (
        local.node_id,
        local.tenant_id,
        local.hub_endpoint,
        local.paired,
    )
}

/// Treat a node id as "this device". Accepts the literal `local`,
/// the empty string (omitted flag), and the device's actual node_id
/// from credentials. Any other value is a remote target, deferred
/// to the federation-Invoke replacement.
fn is_local_target(node_id: &str, local_node_id: &str) -> bool {
    let trimmed = node_id.trim();
    trimmed.is_empty() || trimmed == "local" || trimmed == local_node_id
}

/// Surface the canonical "federation not wired" error from an
/// ability handler. The string mirrors `support::local_invoke`'s
/// helper byte-for-byte so a CLI script that greps the message sees
/// the same wording whether the error came from CLI-side validation
/// (e.g. `--node bogus`) or daemon-side dispatch (here).
fn federation_not_wired(action: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "{action} requires the federation Invoke surface, which was removed by \
         AXON-RFC-001 P1.5 and has not yet been re-published as a \
         federation-tier ability. Local-only operations remain available — \
         see `easynet ability list` for what this node can do without \
         federation. The replacement (Invoke against an Agent ability on \
         the realm) ships in a follow-up; this command will be re-wired \
         without changing its CLI shape when it lands."
    )
}

// ── fleet.list_nodes ─────────────────────────────────────────────

/// List every node visible from this device. v1: just the local
/// node (federation peer enumeration depends on the dead bridge
/// `list_nodes`; will be re-wired through a federation Invoke
/// helper when one ships, at which point this handler fan-outs).
fn list_nodes_handler(_args: Value) -> anyhow::Result<Value> {
    let view = federation_probe::collect_fleet_view();
    let nodes: Vec<Value> = view
        .nodes
        .iter()
        .map(federation_probe::node_to_json)
        .collect();
    Ok(json!({
        "nodes": nodes,
        "federation_view": view.federation_view,
        "federation_view_reason": view.federation_view_reason,
        "resolve_latency_ms": view.resolve_latency_ms,
    }))
}

// ── fleet.describe_node ──────────────────────────────────────────

fn describe_node_handler(args: Value) -> anyhow::Result<Value> {
    let node_id = args
        .get("node_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if node_id.is_empty() {
        anyhow::bail!("fleet.describe_node: `node_id` is required");
    }
    let view = federation_probe::collect_fleet_view();
    let local_id = view
        .nodes
        .iter()
        .find(|n| n.is_self)
        .map(|n| n.node_id.as_str())
        .unwrap_or("local");
    if is_local_target(node_id, local_id) {
        let node = view
            .nodes
            .iter()
            .find(|n| n.is_self)
            .ok_or_else(|| anyhow::anyhow!("fleet.describe_node: local node is unavailable"))?;
        return Ok(federation_probe::node_to_json(node));
    }
    if let Some(node) = view.nodes.iter().find(|n| n.node_id == node_id) {
        return Ok(federation_probe::node_to_json(node));
    }
    let suffix = view
        .federation_view_reason
        .as_deref()
        .map(|reason| format!(" ({reason})"))
        .unwrap_or_default();
    anyhow::bail!("fleet.describe_node: node {node_id:?} not found{suffix}");
}

// ── fleet.remove_node ────────────────────────────────────────────

fn remove_node_handler(args: Value) -> anyhow::Result<Value> {
    let node_id = args
        .get("node_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if node_id.is_empty() {
        anyhow::bail!("fleet.remove_node: `node_id` is required");
    }
    let (local_id, _tenant, _hub, _paired) = local_identity();
    if is_local_target(node_id, &local_id) {
        anyhow::bail!(
            "fleet.remove_node refuses to remove this device (would delete its own \
             pairing). Use `easynet device reset` for that — it is the local \
             side of the same operation."
        );
    }
    Err(federation_not_wired(&format!(
        "removing the remote node {node_id:?}"
    )))
}

// ── fleet.deploy_ability ─────────────────────────────────────────

fn deploy_ability_handler(args: Value) -> anyhow::Result<Value> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("fleet.deploy_ability: `path` is required"))?;
    let node_id = args
        .get("node_id")
        .and_then(Value::as_str)
        .unwrap_or("local")
        .trim();
    let (local_id, _tenant, _hub, _paired) = local_identity();
    if !is_local_target(node_id, &local_id) {
        return Err(federation_not_wired(&format!(
            "deploying an ability to remote node {node_id:?}"
        )));
    }
    // Local deploy: validate the bundle exists. v1 stops there —
    // hot-reloading on the local daemon is already covered by the
    // workspace-abilities loop (drop a TOML into
    // `<agent-root>/abilities/`). The `easynet ability deploy`
    // surface stays intact for future use; today it is documentation
    // for the operator that the ability they pointed at is well-formed.
    let dir = std::path::Path::new(path);
    if !dir.is_dir() {
        anyhow::bail!("fleet.deploy_ability: {path:?} is not a directory");
    }
    let manifest = dir.join("ability.json");
    if !manifest.exists() {
        anyhow::bail!(
            "fleet.deploy_ability: {} does not contain an ability.json",
            dir.display()
        );
    }
    let body = std::fs::read_to_string(&manifest)?;
    let parsed: Value = serde_json::from_str(&body)
        .map_err(|e| anyhow::anyhow!("invalid ability.json at {}: {e}", manifest.display()))?;
    let ability_name = parsed
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("ability.json missing required `name` field"))?;
    Ok(json!({
        "ability_name": ability_name,
        "node_id": local_id,
        "install_id": format!("local-{ability_name}"),
        "state": "ACTIVE",
        "note":
            "Single-node deploy verified the manifest and acknowledged the \
             local registration intent. The federation publish/install/activate \
             saga lights up when the federation Invoke transport returns; until \
             then place agent-owned abilities directly under \
             <agent-root>/abilities/<verb>.ability.toml — the daemon hot-reloads.",
    }))
}

// ── fleet.uninstall_ability ──────────────────────────────────────

fn uninstall_ability_handler(args: Value) -> anyhow::Result<Value> {
    let ability_name = args
        .get("ability_name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("fleet.uninstall_ability: `ability_name` is required"))?;
    let node_id = args
        .get("node_id")
        .and_then(Value::as_str)
        .unwrap_or("local")
        .trim();
    let (local_id, _tenant, _hub, _paired) = local_identity();
    if !is_local_target(node_id, &local_id) {
        return Err(federation_not_wired(&format!(
            "uninstalling ability {ability_name:?} from remote node {node_id:?}"
        )));
    }
    Ok(json!({
        "ability_name": ability_name,
        "node_id": local_id,
        "state": "REMOVED",
        "note":
            "Single-node uninstall acknowledges the request. For agent-owned \
             abilities, delete the corresponding \
             <agent-root>/abilities/<verb>.ability.toml — the daemon hot-reloads. \
             A future federation surface will fan this call out to remote nodes.",
    }))
}

// ── fleet.exec_remote ────────────────────────────────────────────

fn exec_remote_handler(args: Value) -> anyhow::Result<Value> {
    let node_id = args
        .get("node_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if node_id.is_empty() {
        anyhow::bail!("fleet.exec_remote: `node_id` is required");
    }
    let (local_id, _tenant, _hub, _paired) = local_identity();
    if !is_local_target(node_id, &local_id) {
        return Err(federation_not_wired(&format!(
            "running a one-shot command on remote node {node_id:?}"
        )));
    }
    let argv: Vec<String> = args
        .get("command")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if argv.is_empty() {
        anyhow::bail!("fleet.exec_remote: `command` must be a non-empty array of strings");
    }
    let timeout_ms = args
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .filter(|&n| n > 0)
        .unwrap_or(60_000);

    // Dispatch through std::process::Command directly. This is
    // structurally argv (no shell interpretation) — the same
    // injection-safety property `process.exec` and the shell
    // executor enforce. A future refactor can route through
    // `process.exec` instead so policy lives in one place.
    let started = std::time::Instant::now();
    let output = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .output()
        .map_err(|e| anyhow::anyhow!("spawn {:?}: {e}", argv[0]))?;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let _ = Duration::from_millis(timeout_ms); // accepted for forward-compat; not enforced here in v1
    Ok(json!({
        "node_id": local_id,
        "stdout": String::from_utf8_lossy(&output.stdout),
        "stderr": String::from_utf8_lossy(&output.stderr),
        "exit_code": output.status.code().unwrap_or(-1),
        "elapsed_ms": elapsed_ms,
    }))
}

// ── fleet.register_self / fleet.deregister_self ──────────────────
//
// Boot/shutdown lifecycle. The ability invocation is the canonical
// entry point per the ontology; the actual transport work — whether
// a Hub register call, a federation announce, or both — lives
// behind these names. v1 acknowledges the intent and reports the
// current pairing state without performing the legacy
// `bridge.register_node` / `bridge.deregister_node` calls (those
// were P1.5 victims). The federation Invoke replacement, when it
// lands, will populate these handlers without changing the
// ability surface.

fn register_self_handler(_args: Value) -> anyhow::Result<Value> {
    let (node_id, tenant_id, hub, paired) = local_identity();
    Ok(json!({
        "node_id": node_id,
        "tenant_id": tenant_id,
        "hub_endpoint": hub,
        "paired": paired,
        "state": if paired { "REGISTERED" } else { "STANDALONE" },
        "note":
            "Acknowledged. Federation register transport awaits the AXON-RFC-001 \
             P1.5 follow-up; this call is a no-op when no federation peers exist.",
    }))
}

fn deregister_self_handler(_args: Value) -> anyhow::Result<Value> {
    let (node_id, _tenant, _hub, paired) = local_identity();
    Ok(json!({
        "node_id": node_id,
        "paired": paired,
        "state": "DEREGISTERED",
        "note":
            "Acknowledged. `easynet device reset` clears local credentials; the \
             federation deregister fan-out will re-light when the Invoke \
             replacement ships.",
    }))
}

// ── Discovery surfaces ───────────────────────────────────────────

pub fn list_nodes_description() -> &'static str {
    "List device nodes visible from this daemon. The handler resolves \
     the realm directory through federation.resolve and then directly \
     probes each discovered device-profile Agent with observe.health, \
     so callers can distinguish a local-only view, a directory-only view, \
     and a directly reachable peer."
}

pub fn list_nodes_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {}
    })
}

pub fn describe_node_description() -> &'static str {
    "Describe one node by id from the same live federation-backed view \
     used by fleet.list_nodes. Accepts `local`, this device's actual \
     node id, or any resolved peer node id."
}

pub fn describe_node_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["node_id"],
        "properties": {
            "node_id": { "type": "string" }
        }
    })
}

pub fn remove_node_description() -> &'static str {
    "Remove a node from the fleet. Refuses to remove the local device \
     (use `easynet device reset` for that). Remote removal awaits the \
     federation Invoke replacement."
}

pub fn remove_node_input_schema() -> Value {
    describe_node_input_schema()
}

pub fn deploy_ability_description() -> &'static str {
    "Publish an ability bundle to a node. Local target validates the \
     manifest and acknowledges the registration intent. Remote targets \
     defer to the federation Invoke replacement."
}

pub fn deploy_ability_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["path"],
        "properties": {
            "path":    { "type": "string" },
            "node_id": { "type": "string" }
        }
    })
}

pub fn uninstall_ability_description() -> &'static str {
    "Uninstall an ability from a node. Mirrors `fleet.deploy_ability`: \
     local target acknowledged in v1, remote targets queued for the \
     federation Invoke replacement."
}

pub fn uninstall_ability_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["ability_name"],
        "properties": {
            "ability_name": { "type": "string" },
            "node_id":      { "type": "string" },
            "install_id":   { "type": "string" }
        }
    })
}

pub fn exec_remote_description() -> &'static str {
    "Run a one-shot command on a node. Local target dispatches \
     through std::process::Command (argv-only — no shell interpretation). \
     Remote targets defer to the federation Invoke replacement."
}

pub fn exec_remote_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["node_id", "command"],
        "properties": {
            "node_id":    { "type": "string" },
            "command":    { "type": "array", "items": { "type": "string" } },
            "timeout_ms": { "type": "integer", "minimum": 0 }
        }
    })
}

pub fn register_self_description() -> &'static str {
    "Acknowledge this device's pairing state. v1 returns the \
     credentials snapshot without performing the legacy \
     `bridge.register_node` call (P1.5 victim); the federation \
     Invoke replacement will populate the handler in place."
}

pub fn deregister_self_description() -> &'static str {
    "Inverse of fleet.register_self at shutdown. v1 acknowledges \
     intent; federation fan-out lands with the Invoke replacement."
}

pub fn register_self_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {}
    })
}

pub fn deregister_self_input_schema() -> Value {
    register_self_input_schema()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_nodes_returns_at_least_self() {
        let resp = list_nodes_handler(json!({})).unwrap();
        let nodes = resp.get("nodes").and_then(Value::as_array).unwrap();
        assert!(
            nodes.iter().any(|n| n.get("is_self") == Some(&json!(true))),
            "fleet.list_nodes must include the local device entry: {resp}"
        );
        assert!(resp.get("federation_view").is_some());
    }

    #[test]
    fn describe_node_with_local_returns_self_envelope() {
        let resp = describe_node_handler(json!({"node_id": "local"})).unwrap();
        assert_eq!(resp.get("is_self"), Some(&json!(true)));
    }

    #[test]
    fn describe_node_with_remote_returns_not_found() {
        let err = describe_node_handler(json!({"node_id": "some-remote"})).unwrap_err();
        assert!(format!("{err}").contains("not found"));
    }

    #[test]
    fn remove_node_refuses_to_remove_self() {
        let err = remove_node_handler(json!({"node_id": "local"})).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("device reset"),
            "must point at `easynet device reset`; got: {msg}"
        );
    }

    #[test]
    fn deploy_ability_rejects_missing_path() {
        let err = deploy_ability_handler(json!({})).unwrap_err();
        assert!(format!("{err}").contains("path"));
    }

    #[test]
    fn deploy_ability_local_validates_manifest() {
        // path doesn't exist → typed error, not a panic.
        let err = deploy_ability_handler(json!({"path": "/no/such/dir", "node_id": "local"}))
            .unwrap_err();
        assert!(format!("{err}").contains("not a directory"));
    }

    #[test]
    fn exec_remote_local_runs_argv_and_returns_envelope() {
        // Use printf — POSIX, deterministic, available on macOS + Linux.
        let resp = exec_remote_handler(json!({
            "node_id": "local",
            "command": ["printf", "%s", "hello"],
        }))
        .unwrap();
        assert_eq!(resp.get("stdout").and_then(Value::as_str), Some("hello"));
        assert_eq!(resp.get("exit_code"), Some(&json!(0)));
    }

    #[test]
    fn exec_remote_remote_returns_federation_not_wired() {
        let err = exec_remote_handler(json!({
            "node_id": "some-remote",
            "command": ["true"],
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("federation"));
    }

    #[test]
    fn uninstall_ability_local_acknowledges_intent() {
        let resp = uninstall_ability_handler(json!({
            "ability_name": "claude.weather",
            "node_id": "local",
        }))
        .unwrap();
        assert_eq!(resp.get("state").and_then(Value::as_str), Some("REMOVED"));
    }

    #[test]
    fn register_and_deregister_self_acknowledge() {
        let r1 = register_self_handler(json!({})).unwrap();
        assert!(r1.get("state").is_some());
        let r2 = deregister_self_handler(json!({})).unwrap();
        assert_eq!(
            r2.get("state").and_then(Value::as_str),
            Some("DEREGISTERED")
        );
    }
}
