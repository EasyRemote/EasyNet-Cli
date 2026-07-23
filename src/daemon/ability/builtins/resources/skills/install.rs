// EasyNet CLI — skill.install / skill_remove / skill_upgrade
// =================================================================
//
// File: src/daemon/ability/builtins/resources/skills/install.rs
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
// Reuses the runtime package store
// --------------------------------
// `daemon::resources::skills::store::{install_skill, upgrade_skill, remove_skill}`
// is the canonical filesystem implementation. CLI commands call
// these abilities; they do not import or duplicate the store.
//
// Receipt shapes
// --------------
// install: `{ ok: true, record: InstalledSkillProjection }`
//   The projection includes the persisted install fields plus optional
//   response-only fields such as `resource_ura`. Persistence remains owned by
//   `InstallRecord`; ability responses do not mutate persistence JSON.
//
// remove:  `{ ok: true, name, agent }`
//   Idempotency: if the skill isn't present the helper errors; the
//   ability handler surfaces that as a structured error rather than
//   coercing it to a no-op. Callers that want at-least-once
//   semantics (e.g. the Frontend) decide what to do with the typed
//   error.
//
// upgrade: `{ ok: true, record: InstalledSkillProjection }`
//   Same projection shape as install. The helper handles backup + rollback on
//   failure; if the ability returns an error, the on-disk skill is guaranteed
//   to be at the pre-upgrade state.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::daemon::ability::dispatch::AxonAbilityCatalog;
use crate::daemon::federation::read_model::owner_projection::skill_resource_ura;
use crate::daemon::resources::skills::projection::{
    InstalledSkillProjection, SkillRecordResponse, SkillRemoveReceipt,
};
use crate::daemon::resources::skills::store::{install_skill, remove_skill, upgrade_skill};

use crate::daemon::ability::dispatch::OwnerKind;
pub const ABILITY_INSTALL: &str = crate::daemon::ability::names::resources::SKILL_INSTALL;
pub const ABILITY_REMOVE: &str = crate::daemon::ability::names::resources::SKILL_REMOVE;
pub const ABILITY_UPGRADE: &str = crate::daemon::ability::names::resources::SKILL_UPGRADE;

/// Register all three skill-management abilities on the registry.
/// Stateless: no service handle because the helpers read the agent
/// registry from disk on each call (matches the existing CLI
/// behaviour — newly-registered agents are picked up without a
/// daemon restart).
pub fn register(reg: &mut AxonAbilityCatalog) {
    reg.register_rpc_with_owner(
        "skill.install",
        OwnerKind::Device,
        Arc::new(install_handler),
    );
    reg.register_rpc_with_owner("skill.remove", OwnerKind::Device, Arc::new(remove_handler));
    reg.register_rpc_with_owner(
        "skill.upgrade",
        OwnerKind::Device,
        Arc::new(upgrade_handler),
    );
}

/// `skill.install` handler.
///
/// Args: `{ "source": "github:owner/repo[@ref][:subpath]",
///          "agent": "<name>",
///          "pin": "<ref>"? }`
/// Returns: `{ "ok": true, "record": InstalledSkillProjection }`
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

    let record = project_install_record(install_skill(source, agent, pin)?, agent_ura);
    Ok(serde_json::to_value(SkillRecordResponse::ok(record))?)
}

/// `skill.remove` handler.
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
    let resource_ura = agent_ura.and_then(|agent_ura| skill_resource_ura(agent_ura, name));
    Ok(serde_json::to_value(SkillRemoveReceipt::success(
        name,
        agent,
        resource_ura,
    ))?)
}

/// `skill.upgrade` handler.
///
/// Args: `{ "name": "<skill-name>",
///          "agent": "<agent-name>",
///          "to": "<ref>"? }`   // omit = upstream HEAD
/// Returns: `{ "ok": true, "record": InstalledSkillProjection }`
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

    let record = project_install_record(upgrade_skill(name, agent, to)?, agent_ura);
    Ok(serde_json::to_value(SkillRecordResponse::ok(record))?)
}

fn project_install_record(
    record: crate::daemon::resources::skills::store::InstallRecord,
    agent_ura: Option<&str>,
) -> InstalledSkillProjection {
    let resource_ura = agent_ura.and_then(|agent_ura| skill_resource_ura(agent_ura, &record.name));
    InstalledSkillProjection::from_record(record, resource_ura)
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
     v1 supports github: sources. Returns an InstalledSkillProjection (name, agent_id, source, \
     content_hash, size_bytes, installed_at, resource_ura?)."
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
    use crate::daemon::resources::skills::store::{InstallRecord, SkillSource};

    /// All three abilities must register or the daemon-side
    /// dispatch will silently miss them.
    #[test]
    fn registration_makes_all_three_dispatchable() {
        let authority_context =
            crate::daemon::ability::dispatch::AbilityAuthorityContext::for_device_authority_root(
                crate::core::ura::device_ura("localhost", "skill-install-test-device"),
            )
            .expect("build explicit skill-install test Device authority");
        let mut reg =
            AxonAbilityCatalog::new_metadata_only_with_authority_context(authority_context);
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
    fn project_install_record_returns_response_projection_with_resource_ura() {
        let record = InstallRecord {
            name: "alpha".to_string(),
            description: "Alpha skill".to_string(),
            agent_id: "claude".to_string(),
            source: SkillSource {
                kind: "github".to_string(),
                identifier: "owner/repo".to_string(),
                ref_: Some("main".to_string()),
                subpath: None,
            },
            skill_tree_hash: "sha256:abc".to_string(),
            size_bytes: 42,
            installed_at: "2026-04-23T00:00:00Z".to_string(),
            last_checked_at: None,
            upgrade_available: false,
        };

        let projected = project_install_record(record, Some("easynet:///r/acme/agent/u1.claude"));

        assert_eq!(projected.name, "alpha");
        assert_eq!(projected.skill_tree_hash, "sha256:abc");
        assert_eq!(
            projected.resource_ura.as_deref(),
            Some("easynet:///r/acme/resource/agent.u1.claude/skill/alpha")
        );
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
    fn remove_handler_returns_typed_receipt_projection() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let agent_name = "skill-remove-receipt";
        let agent_root = crate::daemon::persistence::config::agents_root().join(agent_name);
        std::fs::create_dir_all(agent_root.join("skills").join("alpha")).expect("skill dir");

        let mut registry = crate::daemon::persistence::agent_registry::AgentRegistry::default();
        let mut agent = crate::daemon::persistence::agent_registry::AgentEntry::new(
            crate::daemon::persistence::agent_registry::AgentType::Codex,
            None,
        );
        agent.root_path = Some(agent_root);
        registry.agents.insert(agent_name.to_string(), agent);
        crate::daemon::persistence::agent_registry::save_agents(&registry).expect("save registry");

        let response = remove_handler(json!({
            "name": "alpha",
            "agent": agent_name,
            "agent_ura": "easynet:///r/acme/agent/u1.claude",
        }))
        .expect("remove");
        let receipt: SkillRemoveReceipt =
            serde_json::from_value(response).expect("typed remove receipt");

        assert!(receipt.ok);
        assert_eq!(receipt.name, "alpha");
        assert_eq!(receipt.agent, agent_name);
        assert_eq!(
            receipt.resource_ura.as_deref(),
            Some("easynet:///r/acme/resource/agent.u1.claude/skill/alpha")
        );
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
