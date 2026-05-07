// EasyNet CLI — fleet.{start_agent, stop_agent} ability handlers
// =================================================================
//
// File: src/runtime/agents/fleet_lifecycle_ability.rs
//
// Per RFC §18, the device-profile owns these abilities. They mirror
// the operator-facing `easynet agent add` / `easynet agent remove`
// CLI subcommands but reach the same registry through Invoke instead
// of stdin parsing — so a remote operator (or another local Agent)
// can manage the fleet without spawning a shell.
//
// Lifecycle model
// ---------------
// LLM sub-agents in EasyNet are *registry rows*, not resident
// processes. Per `~/.easynet/agents.json`, an entry records the
// runtime kind (claude-code / codex / …), the model selector, and
// optional label. The actual claude/codex process is spawned per
// invocation by `agent send` / chat_ability and exits when the
// invocation completes — there is no long-running daemon to start
// or stop.
//
// So `fleet.start_agent` is "register a new agent row + return its
// canonical URA", and `fleet.stop_agent` is "remove the row." The
// ability names match the §18 registry; the verbs map onto today's
// reality.
//
// What lives here
// ---------------
//   * fleet.start_agent — { name, agent_type, model? } →
//                          { canonical_agent_uri, replaced_prior }
//                          replaced_prior=true means the call
//                          overwrote an existing row of the same
//                          name (operator-visible event).
//   * fleet.stop_agent  — { name_or_uri } → { ack: bool }
//                          ack=false when the row didn't exist
//                          (idempotent: callers can retry without
//                          triggering an error).
//
// What does NOT live here
// -----------------------
//   * Workspace cleanup. `easynet agent remove --purge` deletes
//     `~/.easynet/workspaces/<name>/`; the ability deliberately
//     doesn't, so a remote stop_agent can't accidentally wipe an
//     operator's local files. Workspace lifecycle stays under the
//     CLI subcommand.
//   * Process kill signals. There are no resident agent processes
//     today (see Lifecycle model above); a future per-agent
//     long-runner would land its own `fleet.kill_session` ability.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::str::FromStr;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::registry::agents::{self, AgentEntry, AgentType};
use crate::runtime::ability_dispatch::LocalAbilityRegistry;

use crate::runtime::ability_dispatch::OwnerKind;
pub const ABILITY_START_AGENT: &str = "device.fleet.start_agent";
pub const ABILITY_STOP_AGENT: &str = "device.fleet.stop_agent";

pub fn register(reg: &mut LocalAbilityRegistry) {
    reg.register_rpc_with_owner(
        "device.fleet.start_agent",
        OwnerKind::Device,
        Arc::new(|args: Value| start_agent_handler(args)),
    );
    reg.register_rpc_with_owner(
        "device.fleet.stop_agent",
        OwnerKind::Device,
        Arc::new(|args: Value| stop_agent_handler(args)),
    );
}

/// `fleet.start_agent` handler.
///
/// Args: `{ "name": "claude", "agent_type": "claude-code", "model": "sonnet"? }`.
/// Behaviour:
///   1. Validate `name` (non-empty) and parse `agent_type`.
///   2. Load the registry. If `name` already exists, the call
///      replaces it — `replaced_prior=true` so the operator can
///      see they overwrote a row. Same shape as
///      federation.advertise_agent's reply.
///   3. Insert + persist. Return the canonical URA the device-profile
///      would mint for this name (uses the same shape as
///      `local_agents` hosted-agent entries; the actual minting
///      happens at the device-profile boot path — until that
///      pipeline lands here we synthesise a deterministic shape
///      so callers have something to invoke).
fn start_agent_handler(args: Value) -> anyhow::Result<Value> {
    let name = args
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("fleet.start_agent: `name` (non-empty string) required"))?
        .to_string();
    let agent_type_str = args
        .get("agent_type")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("fleet.start_agent: `agent_type` required"))?;
    let agent_type = AgentType::from_str(agent_type_str)?;
    let model = args
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string);

    let mut registry = agents::load_agents().unwrap_or_default();
    let replaced_prior = registry.agents.contains_key(&name);
    registry
        .agents
        .insert(name.clone(), AgentEntry::new(agent_type, model));
    agents::save_agents(&registry)?;

    // Canonical URA shape matches `local_agents` hosted-agent
    // entries (see persistence::local_agents preamble §1.4). We
    // derive the realm from the local-agents file's host device
    // entry when available; pre-join we emit a placeholder realm
    // so the field is well-formed but the operator can tell at a
    // glance the device hasn't joined yet.
    let canonical_uri = derive_canonical_uri(&name);

    Ok(json!({
        "canonical_agent_uri": canonical_uri,
        "replaced_prior": replaced_prior,
    }))
}

/// `fleet.stop_agent` handler.
///
/// Args: `{ "name_or_uri": "claude" | "easynet:///r/<realm>/agent/01LLM-…-claude" }`.
/// Behaviour: remove the registry row. Idempotent — `ack=false` if
/// the row didn't exist; never errors on missing target.
fn stop_agent_handler(args: Value) -> anyhow::Result<Value> {
    let name_or_uri = args
        .get("name_or_uri")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("fleet.stop_agent: `name_or_uri` (non-empty string) required")
        })?;
    // Operators may pass either the bare agent name or its canonical
    // URA. The registry is keyed by name; resolve here so handler
    // semantics are "remove a named row" regardless of input shape.
    let name = registry_name_from_input(name_or_uri);

    let mut registry = agents::load_agents().unwrap_or_default();
    let ack = registry.agents.remove(&name).is_some();
    if ack {
        agents::save_agents(&registry)?;
    }
    Ok(json!({ "ack": ack }))
}

/// Resolve a `name_or_uri` argument to the registry-key form (the
/// bare agent name).
///
/// Three input shapes are accepted:
///   * Bare name (`"claude"`) — returned as-is.
///   * Canonical URA (`"easynet:///r/<realm>/agent/<user>.01LLM-claude"`) —
///     strips the `<user>.` owner segment, then strips the
///     `01LLM-` prefix the device-profile mints.
///   * Anything else (typo, raw id without the prefix) — returned
///     as-is, so the registry lookup misses cleanly with `ack=false`
///     instead of silently matching the wrong row.
///
/// Pure function on `&str`; no I/O. Tested independently below so
/// the parser is auditable in isolation.
fn registry_name_from_input(name_or_uri: &str) -> String {
    // Step 1: peel the URA path-tail. For a bare name ("claude") the
    // input has no slash, so we keep the whole string.
    let tail = name_or_uri
        .rsplit_once('/')
        .map(|(_, t)| t)
        .unwrap_or(name_or_uri);
    // Step 2: peel the `<user>.` owner prefix when the tail is a
    // canonical v4.1.4 agent URI component like `u1.01LLM-claude`.
    let tail = tail.split_once('.').map(|(_, rest)| rest).unwrap_or(tail);
    // Step 3: peel the `01LLM-` device-profile prefix if present.
    // This is the only sanctioned llm-profile prefix today; if a
    // future profile mints a different shape, extend here (NOT at
    // every call site).
    tail.strip_prefix("01LLM-").unwrap_or(tail).to_string()
}

/// Build a placeholder canonical URA for a newly-registered agent.
///
/// The real URA is minted by the device-profile at registry boot —
/// see `runtime::profiles` and `persistence::local_agents`. Until
/// the boot path is wired to call this ability instead of the CLI
/// subcommand, we synthesise a v4.1.4 user-anchored shape
/// (`easynet:///r/<realm>/agent/<user-uuid>.01LLM-<name>`) so a
/// downstream caller has a valid string to put in their UI / logs.
/// Realm + user-uuid are read off the persisted host-device entry;
/// pre-join we use `<unset>` as a visible placeholder.
fn derive_canonical_uri(name: &str) -> String {
    let (realm, user_id) = crate::persistence::local_agents::load()
        .ok()
        .and_then(|f| extract_realm_user_from_uri(&f.host_device_agent_uri))
        .unwrap_or_else(|| ("<unset>".to_string(), "<unset>".to_string()));
    crate::uri::agent_uri(&realm, &user_id, &format!("01LLM-{name}"))
}

/// Extract `(<realm>, <user-uuid>)` from a v4.1.4 host-device agent
/// URA (`easynet:///r/<realm>/agent/<user-uuid>.<agent-id>`). Returns
/// `None` on empty / malformed input so the caller can fall back
/// without spreading parse logic across modules.
fn extract_realm_user_from_uri(uri: &str) -> Option<(String, String)> {
    let parsed = crate::uri::parse_ura(uri).ok()?;
    if parsed.kind != crate::uri::URAKind::Agent {
        return None;
    }
    if parsed.realm.is_empty() || parsed.user_id.is_empty() {
        return None;
    }
    Some((parsed.realm, parsed.user_id))
}

// ── Discovery surfaces ────────────────────────────────────────

pub fn start_agent_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["name", "agent_type"],
        "properties": {
            "name":       { "type": "string", "minLength": 1 },
            "agent_type": { "type": "string",
                            "enum": ["claude-code", "claude", "codex",
                                     "codex-app-server", "codex-appserver"] },
            "model":      { "type": "string" },
        },
        "additionalProperties": false,
    })
}

pub fn start_agent_description() -> &'static str {
    "Register a new LLM sub-agent (claude/codex/…) in the device's \
     agent registry. v1 mirrors `easynet agent add`; the canonical \
     URA in the response follows the §1.4 hosted-Agent shape. \
     replaced_prior=true means the call overwrote an existing row."
}

pub fn stop_agent_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["name_or_uri"],
        "properties": {
            "name_or_uri": { "type": "string", "minLength": 1 },
        },
        "additionalProperties": false,
    })
}

pub fn stop_agent_description() -> &'static str {
    "Remove an LLM sub-agent registry row. Accepts either the agent \
     name or its canonical URA. Idempotent: ack=false when the row \
     didn't exist. Workspace files are deliberately NOT deleted — \
     use `easynet agent remove --purge` for that."
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test fixture: route `~/.easynet/` at a fresh tempdir for the
    /// duration of `f`. Uses the canonical `test_support::HomeGuard`
    /// — same fixture as registry::agents tests and the dispatch suite
    /// — which (a) acquires a process-global mutex so two HomeGuards
    /// never run concurrently and (b) sets `HOME` (the var
    /// `config::home_dir()` actually reads), not `EASYNET_HOME`.
    ///
    /// History: an earlier draft of this fixture set `EASYNET_HOME`
    /// (which `config::home_dir()` ignores) and only PID+nanos-keyed
    /// the tempdir, leaving the env-var racing under parallel tests.
    /// See `docs/rfc/AXON-RFC-001-flake-localization.md` (2026-04-27).
    fn with_isolated_home<F: FnOnce()>(f: F) {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        f();
    }

    #[test]
    fn registration_makes_both_dispatchable() {
        let mut reg = LocalAbilityRegistry::new();
        register(&mut reg);
        assert!(reg.get_rpc(ABILITY_START_AGENT).is_some());
        assert!(reg.get_rpc(ABILITY_STOP_AGENT).is_some());
    }

    #[test]
    fn start_agent_persists_and_returns_canonical_uri() {
        with_isolated_home(|| {
            let resp = start_agent_handler(json!({
                "name": "claude",
                "agent_type": "claude-code",
                "model": "sonnet",
            }))
            .unwrap();
            assert_eq!(resp["replaced_prior"], false);
            let uri = resp["canonical_agent_uri"].as_str().unwrap();
            // v4.1.4: agent URA is user-anchored
            // (`/agent/<user>.<id>`). The pre-join placeholder is
            // literal `<unset>` for both realm and user; the
            // ability-id keeps its `01LLM-` prefix.
            assert!(
                uri.ends_with(".01LLM-claude"),
                "uri must terminate in .01LLM-claude (got: {uri})"
            );
            assert!(
                uri.contains("/agent/"),
                "uri must use the agent role segment (got: {uri})"
            );

            // Round-trip: the registry now has the row.
            let registry = agents::load_agents().unwrap();
            assert!(registry.agents.contains_key("claude"));
        });
    }

    #[test]
    fn start_agent_replaces_existing_row_and_signals_replaced_prior() {
        with_isolated_home(|| {
            // First insertion.
            start_agent_handler(json!({
                "name": "claude",
                "agent_type": "claude-code",
            }))
            .unwrap();
            // Second insertion with the same name — overwrite.
            let resp = start_agent_handler(json!({
                "name": "claude",
                "agent_type": "codex",
            }))
            .unwrap();
            assert_eq!(
                resp["replaced_prior"], true,
                "second insertion of same name MUST flag replaced_prior=true"
            );
        });
    }

    #[test]
    fn start_agent_rejects_missing_name() {
        let err = start_agent_handler(json!({"agent_type": "claude-code"})).unwrap_err();
        assert!(format!("{err}").contains("name"));
    }

    #[test]
    fn start_agent_rejects_unknown_agent_type() {
        let err = start_agent_handler(json!({
            "name": "x",
            "agent_type": "totally-not-a-runtime",
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("unknown agent type"));
    }

    #[test]
    fn stop_agent_by_name_acks_true_and_removes_row() {
        with_isolated_home(|| {
            start_agent_handler(json!({
                "name": "claude",
                "agent_type": "claude-code",
            }))
            .unwrap();

            let resp = stop_agent_handler(json!({"name_or_uri": "claude"})).unwrap();
            assert_eq!(resp["ack"], true);
            assert!(!agents::load_agents().unwrap().agents.contains_key("claude"));
        });
    }

    #[test]
    fn stop_agent_idempotent_returns_ack_false_when_row_missing() {
        with_isolated_home(|| {
            // Never registered; stop should report ack=false (not error).
            let resp = stop_agent_handler(json!({"name_or_uri": "ghost"})).unwrap();
            assert_eq!(resp["ack"], false);
        });
    }

    #[test]
    fn stop_agent_by_uri_extracts_name_tail() {
        with_isolated_home(|| {
            start_agent_handler(json!({
                "name": "claude",
                "agent_type": "claude-code",
            }))
            .unwrap();
            let resp = stop_agent_handler(json!({
                "name_or_uri": "easynet:///r/acme/agent/u1.01LLM-claude"
            }))
            .unwrap();
            assert_eq!(
                resp["ack"], true,
                "URI form must resolve to the same registry row as the bare name"
            );
        });
    }

    #[test]
    fn registry_name_from_input_handles_all_three_shapes() {
        // Bare name: returned as-is.
        assert_eq!(registry_name_from_input("claude"), "claude");
        // Canonical URA: peel path-tail and 01LLM- prefix.
        assert_eq!(
            registry_name_from_input("easynet:///r/acme/agent/u1.01LLM-claude"),
            "claude"
        );
        // Bare id without the 01LLM- prefix: peel only the path-tail.
        // Round-trip mismatch is intentional — the lookup will miss
        // and stop_agent reports ack=false rather than silently
        // matching the wrong row.
        assert_eq!(
            registry_name_from_input("easynet:///r/acme/agent/claude"),
            "claude"
        );
        // Multi-dash agent name: only the prefix is stripped, not
        // every dash-segment. Pin so a future "tidy" rsplit doesn't
        // break "my-claude" → "claude".
        assert_eq!(
            registry_name_from_input("easynet:///r/acme/agent/u1.01LLM-my-claude"),
            "my-claude"
        );
    }

    #[test]
    fn extract_realm_user_from_uri_handles_v414_agent_shape() {
        // v4.1.4 agent URA: r/<realm>/agent/<user>.<agent>
        assert_eq!(
            extract_realm_user_from_uri("easynet:///r/acme/agent/u1.claude"),
            Some(("acme".to_string(), "u1".to_string()))
        );
        // Pre-v4.1.4 single-tail shape is rejected by ParseURA, so
        // this returns None (caller falls back to <unset>).
        assert_eq!(
            extract_realm_user_from_uri("easynet:///r/acme/agent/01DEV"),
            None
        );
        assert_eq!(extract_realm_user_from_uri(""), None);
        assert_eq!(extract_realm_user_from_uri("not-a-uri"), None);
        // Empty realm is rejected.
        assert_eq!(
            extract_realm_user_from_uri("easynet:///r//agent/u1.claude"),
            None,
            "empty realm must NOT be considered valid"
        );
    }

    #[test]
    fn input_schemas_have_required_fields_pinned() {
        let s = start_agent_input_schema();
        let req = s["required"].as_array().unwrap();
        assert!(req.iter().any(|v| v == "name"));
        assert!(req.iter().any(|v| v == "agent_type"));
        assert_eq!(s["additionalProperties"], false);

        let s = stop_agent_input_schema();
        let req = s["required"].as_array().unwrap();
        assert!(req.iter().any(|v| v == "name_or_uri"));
        assert_eq!(s["additionalProperties"], false);
    }
}
