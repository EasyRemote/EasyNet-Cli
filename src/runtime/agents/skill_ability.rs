// EasyNet CLI — skill inventory filesystem walk
// =====================================================================
//
// File: src/runtime/agents/skill_ability.rs
// Description: Shared filesystem walk behind `skill.list`.
//              This module is not registered as a standalone system
//              ability; it exists so `skill.list` and CLI
//              tests use one source of truth for installed skills.
//
// Why "skill list" is an ability, not a separate gRPC RPC
// -------------------------------------------------------
// Architecturally, every discoverable resource on a node should
// flow through the same channel:
//
//   ListMCPTools (federation discovery) → CallMCPTool (execution)
//
// MCP tools, sessions (`session.list`), schedules
// (`schedule.list`), discuss rooms (`discuss.create`),
// permission requests (`consent.subscribe`), and now
// skills all use this single pattern. A fresh RPC per resource type
// would force the proto, the FFI bridge, the SDK, and the backend to
// each grow a parallel parser; routing through abilities reuses the
// machinery once.
//
// Wire shape
// ----------
// The exported helper returns `{ "items": [InstallRecord, ...] }`
// where each item matches the on-wire `InstalledSkill` schema. Each
// item carries:
//   * name, description, agent_id, resource_ura
//   * source = { kind, identifier, ref?, subpath? }
//   * content_hash (empty for global-pool skills)
//   * size_bytes, installed_at, last_checked_at?, upgrade_available
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde_json::{json, Value};

use crate::registry::agents;

/// Crate-internal entry for the `skill.list` walk.
pub(crate) fn list_handler_for_args(args: Value) -> anyhow::Result<Value> {
    list_handler(args)
}

/// Skill inventory handler.
///
/// Args:
/// ```json
/// {
///   "owner_agent_id": "<local agent name>"?,
///   "agent_ura": "<canonical owner Agent URA>"?,
///   "subject_ura": "<owner Agent URA or skill package Resource URA>"?
/// }
/// ```
///
/// `owner_agent_id` is a local workspace selector. `agent_ura` and
/// `subject_ura` are canonical query scopes. If both forms are
/// supplied they must resolve to the same hosted agent.
///
/// Returns: `{ "items": [InstalledSkill, ...] }`. The InstalledSkill
/// shape matches `EasyNet/backend/internal/types/custom_types.go`
/// `InstalledSkill` so the backend can decode the response and
/// forward to the frontend without re-shaping fields. Each item
/// carries `agent_id` even though the request was filtered, because
/// the union of pools is fan-out per agent and the backend's wire
/// schema requires the field on every row.
fn list_handler(args: Value) -> anyhow::Result<Value> {
    let registry = agents::load_agents()?;
    let local_agents = crate::persistence::local_agents::load().ok();
    let scope = SkillListScope::from_args(&args, local_agents.as_ref())?;

    // Collect rows in the same shape `easynet skill list --json`
    // emitted historically — the backend already knows how to read
    // that shape (see `cliInstalledRow` in
    // backend/internal/logic/skill/listInstalledLogic.go), and this
    // ability now becomes the definitive source.
    use crate::runtime::skill_store::{
        global_skill_pools_for, read_install_record, scan_global_pool_into, InstallRecord,
    };
    let mut rows: Vec<InstallRecord> = Vec::new();

    for (name, entry) in &registry.agents {
        if let Some(filter) = &scope.owner_agent_id {
            if filter != name {
                continue;
            }
        }

        // Source 1 — EasyNet-managed installs. Path layout is owned
        // by the agent type and resolved through
        // `managed_skill_dir_for_agent_type`, so listing and publish
        // code cannot silently grow another compatibility search path.
        let root = entry
            .root_path
            .clone()
            .unwrap_or_else(|| crate::persistence::config::agents_root().join(name));
        let skills_dir = managed_skill_dir_for_agent_type(&root, entry.agent_type);
        if skills_dir.exists() {
            if let Ok(read) = std::fs::read_dir(&skills_dir) {
                for dir_entry in read.flatten() {
                    let record_path = dir_entry.path().join(".easynet").join("install.json");
                    if !record_path.exists() {
                        continue;
                    }
                    match read_install_record(&record_path) {
                        Ok(r) => rows.push(r),
                        Err(e) => {
                            // Per-row failure must not blank the
                            // whole list — log and skip. The
                            // operator sees the warning in the
                            // daemon log; the rest of the list
                            // surfaces normally.
                            let path_display = format!("{}", dir_entry.path().display());
                            let err_msg = format!("{e}");
                            crate::op_event!(
                                component = skill_list,
                                kind = entry_skipped,
                                level = "warn",
                                path = path_display,
                                error = err_msg,
                            );
                        }
                    }
                }
            }
        }

        // Source 2 — agent-native global pools (~/.claude/skills,
        // ~/.agents/skills). These are populated by external tooling
        // and have no install.json; metadata is synthesised from
        // SKILL.md frontmatter (or directory name fallback) inside
        // scan_global_pool_into.
        for (label, pool_dir) in global_skill_pools_for(entry.agent_type) {
            scan_global_pool_into(name, label, &pool_dir, &mut rows);
        }
    }
    if let Some(skill_name) = &scope.skill_name {
        rows.retain(|row| row.name == *skill_name);
    }

    // Serialise InstallRecord directly — its serde derive emits the
    // wire shape backend already speaks (content_hash via
    // #[serde(rename)], etc.). Building the items array via
    // serde_json::to_value preserves field ordering deterministically
    // for downstream byte-stable comparisons.
    let items: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            let mut value = serde_json::to_value(&r).unwrap_or(Value::Null);
            if let Some(resource_ura) = scoped_skill_resource_ura(
                local_agents.as_ref(),
                scope.agent_ura_for_row(&r.agent_id),
                &r.agent_id,
                &r.name,
            ) {
                if let Some(obj) = value.as_object_mut() {
                    obj.insert("resource_ura".to_string(), json!(resource_ura));
                }
            }
            value
        })
        .collect();

    Ok(json!({ "items": items }))
}

fn managed_skill_dir_for_agent_type(
    root: &std::path::Path,
    agent_type: crate::registry::agents::AgentType,
) -> std::path::PathBuf {
    match agent_type {
        crate::registry::agents::AgentType::ClaudeCode => root.join(".claude").join("skills"),
        crate::registry::agents::AgentType::Codex
        | crate::registry::agents::AgentType::CodexAppServer => root.join("skills"),
    }
}

struct SkillListScope {
    owner_agent_id: Option<String>,
    agent_ura: Option<String>,
    skill_name: Option<String>,
}

impl SkillListScope {
    fn from_args(
        args: &Value,
        local_agents: Option<&crate::persistence::local_agents::LocalAgentsFile>,
    ) -> anyhow::Result<Self> {
        let object = args
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("skill.list: args must be a JSON object"))?;
        for key in object.keys() {
            match key.as_str() {
                "owner_agent_id" | "agent_ura" | "subject_ura" => {}
                other => anyhow::bail!("skill.list: unsupported field `{other}`"),
            }
        }

        let owner_agent_id = string_arg(object, "owner_agent_id");
        let agent_ura = string_arg(object, "agent_ura");
        let subject = string_arg(object, "subject_ura")
            .map(|subject| {
                crate::runtime::owner_projection::project_agent_skill_subject(&subject)
                    .map_err(|e| anyhow::anyhow!("skill.list: {e}"))
            })
            .transpose()?;
        let scoped_agent_ura = merge_agent_scope(agent_ura, subject.as_ref())?;
        let scoped_owner = scoped_agent_ura
            .as_deref()
            .map(|ura| owner_name_for_agent_ura(local_agents, ura))
            .transpose()?;
        if let (Some(owner), Some(scoped_owner)) = (&owner_agent_id, &scoped_owner) {
            if owner != scoped_owner {
                anyhow::bail!(
                    "skill.list: owner_agent_id {owner:?} does not match agent_ura/subject_ura owner {scoped_owner:?}"
                );
            }
        }

        Ok(Self {
            owner_agent_id: scoped_owner.or(owner_agent_id),
            agent_ura: scoped_agent_ura,
            skill_name: subject.and_then(|scope| scope.skill_name),
        })
    }

    fn agent_ura_for_row(&self, row_agent_id: &str) -> Option<&str> {
        match (&self.owner_agent_id, &self.agent_ura) {
            (Some(owner), Some(agent_ura)) if owner == row_agent_id => Some(agent_ura.as_str()),
            (None, Some(agent_ura)) => Some(agent_ura.as_str()),
            _ => None,
        }
    }
}

fn string_arg(object: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn merge_agent_scope(
    agent_ura: Option<String>,
    subject: Option<&crate::runtime::owner_projection::AgentSkillSubjectProjection>,
) -> anyhow::Result<Option<String>> {
    match (agent_ura, subject) {
        (Some(agent_ura), Some(subject)) if agent_ura != subject.agent_ura => {
            anyhow::bail!("skill.list: agent_ura and subject_ura owner must match")
        }
        (Some(agent_ura), _) => Ok(Some(agent_ura)),
        (None, Some(subject)) => Ok(Some(subject.agent_ura.clone())),
        (None, None) => Ok(None),
    }
}

fn owner_name_for_agent_ura(
    local_agents: Option<&crate::persistence::local_agents::LocalAgentsFile>,
    agent_ura: &str,
) -> anyhow::Result<String> {
    let Some(local_agents) = local_agents else {
        anyhow::bail!("skill.list: agent_ura requires local-agents.json to resolve local owner");
    };
    local_agents
        .hosted_agents
        .iter()
        .find(|entry| entry.agent_ura == agent_ura)
        .map(|entry| entry.name.clone())
        .ok_or_else(|| anyhow::anyhow!("skill.list: agent_ura {agent_ura:?} is not hosted here"))
}

fn scoped_skill_resource_ura(
    local_agents: Option<&crate::persistence::local_agents::LocalAgentsFile>,
    explicit_agent_ura: Option<&str>,
    agent_name: &str,
    skill_name: &str,
) -> Option<String> {
    let agent_ura = explicit_agent_ura
        .map(str::to_string)
        .or_else(|| hosted_agent_ura(local_agents?, agent_name))?;
    crate::runtime::owner_projection::skill_resource_ura(&agent_ura, skill_name)
}

fn hosted_agent_ura(
    local_agents: &crate::persistence::local_agents::LocalAgentsFile,
    agent_name: &str,
) -> Option<String> {
    local_agents
        .hosted_agents
        .iter()
        .find(|entry| entry.name == agent_name)
        .map(|entry| entry.agent_ura.clone())
}

/// JSON Schema for the input.
pub fn list_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "owner_agent_id": {
                "type": "string",
                "description": "Restrict the list to skills owned by this agent. Absent = every registered agent."
            },
            "agent_ura": {
                "type": "string",
                "description": "Canonical owner Agent URA. Filters to that hosted agent and derives resource_ura on returned rows."
            },
            "subject_ura": {
                "type": "string",
                "description": "Owner Agent URA or skill package Resource URA. A skill Resource URA filters to that single skill."
            }
        },
        "additionalProperties": false,
    })
}

/// Human-readable blurb for discovery surfaces.
pub fn list_description() -> &'static str {
    "List installed skills across registered agents. Combines managed skill installs \
     with agent-native global pools and returns InstalledSkill rows."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_makes_list_dispatchable() {
        let res = list_handler_for_args(json!({})).expect("list helper ok");
        assert!(res.get("items").and_then(Value::as_array).is_some());
    }

    #[test]
    fn list_input_schema_is_object_with_optional_agent_id() {
        // The schema is the contract a UI renders. Pin the keys so a typo trips here.
        let s = list_input_schema();
        assert_eq!(s["type"], "object");
        assert_eq!(s["additionalProperties"], false);
        let props = s["properties"].as_object().expect("properties is object");
        assert!(props.contains_key("owner_agent_id"));
        assert!(props.contains_key("agent_ura"));
        assert!(props.contains_key("subject_ura"));
        assert_eq!(props["owner_agent_id"]["type"], "string");
        // No `required` array — owner_agent_id is optional (omitted = list
        // across every agent).
        assert!(s.get("required").is_none());
    }

    #[test]
    fn managed_skill_dir_for_claude_code_uses_native_project_dir_only() {
        let root = std::path::Path::new("/tmp/agent-root");
        let dir =
            managed_skill_dir_for_agent_type(root, crate::registry::agents::AgentType::ClaudeCode);
        assert_eq!(dir, root.join(".claude").join("skills"));
        assert_ne!(
            dir,
            root.join("skills"),
            "claude-code must not scan retired root-level skills directory"
        );
    }

    #[test]
    fn managed_skill_dir_for_codex_profiles_uses_agent_root_skills() {
        let root = std::path::Path::new("/tmp/agent-root");
        for agent_type in [
            crate::registry::agents::AgentType::Codex,
            crate::registry::agents::AgentType::CodexAppServer,
        ] {
            let dir = managed_skill_dir_for_agent_type(root, agent_type);
            assert_eq!(dir, root.join("skills"));
        }
    }
}
