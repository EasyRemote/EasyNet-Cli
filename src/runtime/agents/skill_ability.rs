// EasyNet CLI — skill inventory filesystem walk
// =====================================================================
//
// File: src/runtime/agents/skill_ability.rs
// Description: Shared filesystem walk behind `device.skill.list`.
//              This module is not registered as a standalone system
//              ability; it exists so `device.skill.list` and CLI
//              tests use one source of truth for installed skills.
//
// Why "skill list" is an ability, not a separate gRPC RPC
// -------------------------------------------------------
// Architecturally, every discoverable resource on a node should
// flow through the same channel:
//
//   ListMCPTools (federation discovery) → CallMCPTool (execution)
//
// MCP tools, sessions (`device.session.list`), schedules
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

/// Crate-internal entry for the `device.skill.list` walk.
pub(crate) fn list_handler_for_args(args: Value) -> anyhow::Result<Value> {
    list_handler(args)
}

/// Skill inventory handler.
///
/// Args: `{ "owner_agent_id": "<name>"? }` — when present, filter to
/// skills owned by that agent; absent or empty = list across every
/// registered agent.
///
/// Returns: `{ "items": [InstalledSkill, ...] }`. The InstalledSkill
/// shape matches `EasyNet/backend/internal/types/custom_types.go`
/// `InstalledSkill` so the backend can decode the response and
/// forward to the frontend without re-shaping fields. Each item
/// carries `agent_id` even though the request was filtered, because
/// the union of pools is fan-out per agent and the backend's wire
/// schema requires the field on every row.
fn list_handler(args: Value) -> anyhow::Result<Value> {
    let agent_filter = args
        .as_object()
        .and_then(|o| o.get("owner_agent_id").or_else(|| o.get("agent_id")))
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let registry = agents::load_agents()?;
    let local_agents = crate::persistence::local_agents::load().ok();
    let explicit_agent_ura = args
        .as_object()
        .and_then(|o| o.get("agent_ura"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    // Collect rows in the same shape `easynet skill list --json`
    // emitted historically — the backend already knows how to read
    // that shape (see `cliInstalledRow` in
    // backend/internal/logic/skill/listInstalledLogic.go), and this
    // ability now becomes the definitive source.
    use crate::facade::cli::skill::{
        global_skill_pools_for, read_install_record, scan_global_pool_into, InstallRecord,
    };
    let mut rows: Vec<InstallRecord> = Vec::new();

    for (name, entry) in &registry.agents {
        if let Some(filter) = &agent_filter {
            if filter != name {
                continue;
            }
        }

        // Source 1 — EasyNet-managed installs.
        //
        // Path layout depends on agent type. For claude-code agents
        // we publish skills under `<root>/.claude/skills/<name>/`
        // (matching Claude Code's project-local skill convention so
        // the running `claude` subprocess auto-loads them). For
        // codex agents we use `<root>/skills/<name>/` because codex
        // has no native project-local skill convention. Either
        // way, the install record at
        // `<dir>/.easynet/install.json` carries full provenance.
        //
        // We also scan the legacy `<root>/skills/` location for
        // claude-code agents — earlier published skills (before the
        // 2026-04-29 fix) live there. New publishes write to
        // `.claude/skills/`; the legacy walk lets `easynet skill
        // list` keep surfacing them until they're republished.
        let root = entry
            .root_path
            .clone()
            .unwrap_or_else(|| crate::persistence::config::agents_root().join(name));
        let mut skill_dirs: Vec<std::path::PathBuf> = Vec::new();
        match entry.agent_type {
            crate::registry::agents::AgentType::ClaudeCode => {
                skill_dirs.push(root.join(".claude").join("skills"));
                skill_dirs.push(root.join("skills")); // legacy, pre-fix
            }
            crate::registry::agents::AgentType::Codex
            | crate::registry::agents::AgentType::CodexAppServer => {
                skill_dirs.push(root.join("skills"));
            }
        }
        for skills_dir in &skill_dirs {
            if !skills_dir.exists() {
                continue;
            }
            if let Ok(read) = std::fs::read_dir(skills_dir) {
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

    // Serialise InstallRecord directly — its serde derive emits the
    // wire shape backend already speaks (content_hash via
    // #[serde(rename)], etc.). Building the items array via
    // serde_json::to_value preserves field ordering deterministically
    // for downstream byte-stable comparisons.
    let items: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            let mut value = serde_json::to_value(&r).unwrap_or(Value::Null);
            if let Some(resource_ura) = skill_resource_ura(
                local_agents.as_ref(),
                explicit_agent_ura.as_deref(),
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

fn skill_resource_ura(
    local_agents: Option<&crate::persistence::local_agents::LocalAgentsFile>,
    explicit_agent_ura: Option<&str>,
    agent_name: &str,
    skill_name: &str,
) -> Option<String> {
    let agent_ura = explicit_agent_ura
        .map(str::to_string)
        .or_else(|| hosted_agent_ura(local_agents?, agent_name))?;
    let parsed = crate::ura::parse_ura(&agent_ura).ok()?;
    if parsed.kind != crate::ura::URAKind::Agent {
        return None;
    }
    Some(crate::ura::resource_dot_ura(
        &parsed.realm,
        &format!("agent.{}.{}", parsed.user_id, parsed.agent_id),
        &format!("skill/{skill_name}"),
    ))
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
                "description": "Canonical agent URA for the selected owner; used to derive resource_ura on returned rows."
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
        assert_eq!(props["owner_agent_id"]["type"], "string");
        // No `required` array — owner_agent_id is optional (omitted = list
        // across every agent).
        assert!(s.get("required").is_none());
    }
}
