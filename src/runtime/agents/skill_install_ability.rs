// EasyNet CLI — device.skill.install / skill_remove / skill_upgrade
// =================================================================
//
// File: src/runtime/agents/skill_install_ability.rs
//
// device-profile abilities the daemon executes locally to manage
// the on-disk skill set for one of its agents. Pre-RFC-001 the
// EasyNet backend reached these operations via a generic
// `ExecCommand("easynet skill install --json")` shell-out — that
// path is forbidden by AXIOM §A3 (no generic shell-out as a
// node-to-node primitive) and was retired in P5-fix-5b. The three
// abilities here are the per-purpose replacement: each privileged
// operation gets its own ability with a typed input + typed
// receipt, all unary signed Invoke (no streaming dependency).
//
// Why these belong on device-profile, not hub or llm-profile
// ----------------------------------------------------------
// Skills live on the device's filesystem under
// `<agent-root>/skills/<dir>/`. The device-profile is the only
// Agent that owns the on-disk layout for its host's agents; any
// other Agent that wanted to install a skill would have to either
// (a) call back to a device-profile (defeats the layering), or
// (b) own its own copy of the on-disk semantics (duplicates the
// install/upgrade/rollback logic the CLI already has).
//
// Reuses the pure helpers
// -----------------------
// `facade::cli::skill::{install_skill, upgrade_skill, remove_skill}`
// are the typed-result helpers extracted from the existing CLI
// `easynet skill install/upgrade/remove` commands. The ability
// handlers in this file are 5-line wrappers around them — same
// logic, no divergence between operator-CLI and ability invocation
// surfaces.
//
// Receipt shapes
// --------------
// install: `{ ok: true, record: InstallRecord }`
//   InstallRecord includes name, agent_id, source { kind,
//   identifier, ref?, subpath? }, skill_tree_hash (sha256 over the
//   installed tree excluding .easynet/), size_bytes, installed_at.
//   Backend forwards this verbatim to the Frontend.
//
// remove:  `{ ok: true, name, agent }`
//   Idempotency: if the skill isn't present the helper errors; the
//   ability handler surfaces that as a structured error rather than
//   coercing it to a no-op. Callers that want at-least-once
//   semantics (e.g. the Frontend) decide what to do with the typed
//   error.
//
// upgrade: `{ ok: true, record: InstallRecord }`
//   Same shape as install. The helper handles backup + rollback on
//   failure; if the ability returns an error, the on-disk skill is
//   guaranteed to be at the pre-upgrade state.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::facade::cli::skill::{install_skill, remove_skill, upgrade_skill};
use crate::runtime::ability_dispatch::LocalAbilityRegistry;

use crate::runtime::ability_dispatch::OwnerKind;
pub const ABILITY_INSTALL: &str = "device.skill.install";
pub const ABILITY_REMOVE: &str = "device.skill.remove";
pub const ABILITY_UPGRADE: &str = "device.skill.upgrade";

/// Register all three skill-management abilities on the registry.
/// Stateless: no service handle because the helpers read the agent
/// registry from disk on each call (matches the existing CLI
/// behaviour — newly-registered agents are picked up without a
/// daemon restart).
pub fn register(reg: &mut LocalAbilityRegistry) {
    reg.register_rpc_with_owner(
        "device.skill.install",
        OwnerKind::Device,
        Arc::new(install_handler),
    );
    reg.register_rpc_with_owner(
        "device.skill.remove",
        OwnerKind::Device,
        Arc::new(remove_handler),
    );
    reg.register_rpc_with_owner(
        "device.skill.upgrade",
        OwnerKind::Device,
        Arc::new(upgrade_handler),
    );
}

/// `device.skill.install` handler.
///
/// Args: `{ "source": "github:owner/repo[@ref][:subpath]",
///          "agent": "<name>",
///          "pin": "<ref>"? }`
/// Returns: `{ "ok": true, "record": InstallRecord + { resource_ura } }`
fn install_handler(args: Value) -> anyhow::Result<Value> {
    let source = args
        .get("source")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("`source` is required"))?;
    let agent = args
        .get("agent")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("`agent` is required"))?;
    let pin = args.get("pin").and_then(Value::as_str);
    let agent_ura = args.get("agent_ura").and_then(Value::as_str);

    let record = install_skill(source, agent, pin)?;
    Ok(json!({ "ok": true, "record": record_with_resource_ura(record, agent_ura) }))
}

/// `device.skill.remove` handler.
///
/// Args: `{ "name": "<skill-name>", "agent": "<agent-name>" }`
/// Returns: `{ "ok": true, "name": "...", "agent": "..." }`
fn remove_handler(args: Value) -> anyhow::Result<Value> {
    let name = args
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("`name` is required"))?;
    let agent = args
        .get("agent")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("`agent` is required"))?;
    let agent_ura = args.get("agent_ura").and_then(Value::as_str);

    remove_skill(name, agent)?;
    let mut receipt = json!({
        "ok": true,
        "name": name,
        "agent": agent,
    });
    if let Some(uri) = agent_ura.and_then(|agent_ura| skill_resource_ura(agent_ura, name)) {
        receipt["resource_ura"] = json!(uri);
    }
    Ok(receipt)
}

/// `device.skill.upgrade` handler.
///
/// Args: `{ "name": "<skill-name>",
///          "agent": "<agent-name>",
///          "to": "<ref>"? }`   // omit = upstream HEAD
/// Returns: `{ "ok": true, "record": InstallRecord }`
fn upgrade_handler(args: Value) -> anyhow::Result<Value> {
    let name = args
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("`name` is required"))?;
    let agent = args
        .get("agent")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("`agent` is required"))?;
    let to = args.get("to").and_then(Value::as_str);
    let agent_ura = args.get("agent_ura").and_then(Value::as_str);

    let record = upgrade_skill(name, agent, to)?;
    Ok(json!({ "ok": true, "record": record_with_resource_ura(record, agent_ura) }))
}

fn record_with_resource_ura(
    record: crate::facade::cli::skill::InstallRecord,
    agent_ura: Option<&str>,
) -> Value {
    let mut value = serde_json::to_value(&record).unwrap_or(Value::Null);
    if let Some(uri) = agent_ura.and_then(|agent_ura| skill_resource_ura(agent_ura, &record.name)) {
        if let Some(obj) = value.as_object_mut() {
            obj.insert("resource_ura".to_string(), json!(uri));
        }
    }
    value
}

fn skill_resource_ura(agent_ura: &str, skill_name: &str) -> Option<String> {
    let parsed = crate::ura::parse_ura(agent_ura).ok()?;
    if parsed.kind != crate::ura::URAKind::Agent {
        return None;
    }
    Some(crate::ura::resource_dot_ura(
        &parsed.realm,
        &format!("agent.{}.{}", parsed.user_id, parsed.agent_id),
        &format!("skill/{skill_name}"),
    ))
}

// ── Discovery surfaces (input schemas + descriptions) ─────────────

pub fn install_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "source": {
                "type": "string",
                "description": "Skill source URL: github:<owner>/<repo>[@<ref>][:<subpath>]"
            },
            "agent": {
                "type": "string",
                "description": "Agent name that will own this skill"
            },
            "agent_ura": {
                "type": "string",
                "description": "Canonical agent URA for the owning agent; used to derive returned resource_ura."
            },
            "pin": {
                "type": "string",
                "description": "Override the ref in the source URL with a concrete tag/SHA"
            },
        },
        "required": ["source", "agent"],
        "additionalProperties": false,
    })
}

pub fn remove_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": {"type": "string", "description": "Skill name as installed under <agent-root>/skills/<name>/"},
            "agent": {"type": "string", "description": "Agent that owns the skill"},
            "agent_ura": {"type": "string", "description": "Canonical agent URA for audit context."},
        },
        "required": ["name", "agent"],
        "additionalProperties": false,
    })
}

pub fn upgrade_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": {"type": "string", "description": "Skill name to upgrade"},
            "agent": {"type": "string", "description": "Agent that owns the skill"},
            "agent_ura": {
                "type": "string",
                "description": "Canonical agent URA for the owning agent; used to derive returned resource_ura."
            },
            "to": {
                "type": "string",
                "description": "Target ref (tag/SHA/branch). Omit for upstream HEAD."
            },
        },
        "required": ["name", "agent"],
        "additionalProperties": false,
    })
}

pub fn install_description() -> &'static str {
    "Install a skill from a marketplace source into an agent's skills/ directory. \
     v1 supports github: sources. Returns the InstallRecord (name, agent_id, source, \
     skill_tree_hash, size_bytes, installed_at)."
}

pub fn remove_description() -> &'static str {
    "Remove an installed skill from an agent. Errors when the skill isn't present \
     so callers can decide whether to treat that as success."
}

pub fn upgrade_description() -> &'static str {
    "Upgrade an installed skill to a target ref (or upstream HEAD). The current \
     skill dir is backed up before the new version is fetched and installed; \
     if anything fails the backup is restored."
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All three abilities must register or the daemon-side
    /// dispatch will silently miss them.
    #[test]
    fn registration_makes_all_three_dispatchable() {
        let mut reg = LocalAbilityRegistry::new();
        register(&mut reg);
        assert!(reg.get_rpc(ABILITY_INSTALL).is_some());
        assert!(reg.get_rpc(ABILITY_REMOVE).is_some());
        assert!(reg.get_rpc(ABILITY_UPGRADE).is_some());
    }

    #[test]
    fn install_handler_rejects_missing_source() {
        let err = install_handler(json!({"agent": "claude"})).unwrap_err();
        assert!(format!("{err}").contains("`source`"));
    }

    #[test]
    fn install_handler_rejects_missing_agent() {
        let err = install_handler(json!({"source": "github:x/y"})).unwrap_err();
        assert!(format!("{err}").contains("`agent`"));
    }

    #[test]
    fn remove_handler_rejects_missing_name() {
        let err = remove_handler(json!({"agent": "claude"})).unwrap_err();
        assert!(format!("{err}").contains("`name`"));
    }

    #[test]
    fn remove_handler_rejects_missing_agent() {
        let err = remove_handler(json!({"name": "alive-video"})).unwrap_err();
        assert!(format!("{err}").contains("`agent`"));
    }

    #[test]
    fn upgrade_handler_rejects_missing_name() {
        let err = upgrade_handler(json!({"agent": "claude"})).unwrap_err();
        assert!(format!("{err}").contains("`name`"));
    }

    #[test]
    fn upgrade_handler_rejects_missing_agent() {
        let err = upgrade_handler(json!({"name": "alive-video"})).unwrap_err();
        assert!(format!("{err}").contains("`agent`"));
    }

    /// Pin the JSON-schema shape for each ability so a UI rendering
    /// the catalog can rely on the property keys.
    #[test]
    fn input_schemas_have_required_arrays() {
        let s = install_input_schema();
        let req = s["required"].as_array().expect("install requires");
        assert!(req.iter().any(|v| v == "source"));
        assert!(req.iter().any(|v| v == "agent"));

        let s = remove_input_schema();
        let req = s["required"].as_array().expect("remove requires");
        assert!(req.iter().any(|v| v == "name"));
        assert!(req.iter().any(|v| v == "agent"));

        let s = upgrade_input_schema();
        let req = s["required"].as_array().expect("upgrade requires");
        assert!(req.iter().any(|v| v == "name"));
        assert!(req.iter().any(|v| v == "agent"));
        // `to` MUST NOT be required — omitted = upstream HEAD.
        assert!(!req.iter().any(|v| v == "to"));
    }
}
