// EasyNet CLI — fleet.* (skill enumeration as a system ability)
// =====================================================================
//
// File: src/runtime/system/skill_ability.rs
// Description: Surfaces installed skills (marketplace + agent-native
//              global pools) through the unified ability dispatch
//              channel. Replaces the SSH-based path the EasyNet
//              backend used to take (`ExecCommand("easynet skill
//              list --json")`), which was disabled by default for
//              safety reasons and broke whenever PATH/HOME differed
//              for the exec context.
//
// Why "skill list" is an ability, not a separate gRPC RPC
// -------------------------------------------------------
// Architecturally, every discoverable resource on a node should
// flow through the same channel:
//
//   ListMCPTools (federation discovery) → CallMCPTool (execution)
//
// MCP tools, sessions (`fleet.list_sessions`), schedules
// (`schedule.list`), discuss rooms (`discuss.create`),
// permission requests (`consent.subscribe`), and now
// skills all use this single pattern. A fresh RPC per resource type
// would force the proto, the FFI bridge, the SDK, and the backend to
// each grow a parallel parser; routing through abilities reuses the
// machinery once.
//
// v1 verbs
// --------
// Only `fleet.list_abilities` lands in this commit — that is the verb
// the EasyNet frontend's Skills page needs. Future verbs (`install`,
// `remove`, `upgrade`) will be wired in subsequent commits and reuse
// the same registration template; the CLI commands
// (`easynet skill install/remove/upgrade`) keep their existing
// implementations and simply route through the ability when called
// from frontends.
//
// Wire shape
// ----------
// The ability returns a JSON object `{ "items": [InstallRecord, ...] }`
// where each item matches the on-wire `InstalledSkill` schema the
// EasyNet backend already speaks (so backend doesn't need to
// re-shape — it just forwards). Specifically each item carries:
//   * name, description, agent_id
//   * source = { kind, identifier, ref?, subpath? }
//   * content_hash (empty for global-pool skills)
//   * size_bytes, installed_at, last_checked_at?, upgrade_available
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::registry::agents;
use crate::runtime::ability_dispatch::LocalAbilityRegistry;

/// The wire-level ability name. Pinned because backend + frontend
/// query it by string; a rename would break both repos at once.
pub const ABILITY_LIST: &str = "fleet.list_abilities";

/// Register every skill verb on the registry. v1 only ships `list`;
/// the other verbs (`install`, `remove`, `upgrade`) plug in here
/// without changing the call shape.
///
/// Stateless: takes no service handle because skill enumeration is a
/// pure file-system walk over `~/.easynet/agents.json` plus the
/// per-agent global pools. Symmetric with `ping::register` — no
/// captured state, no per-request bookkeeping.
pub fn register(reg: &mut LocalAbilityRegistry) {
    reg.register_rpc(ABILITY_LIST, Arc::new(list_handler));
}

/// Crate-internal entry for the `fleet.list_abilities` walk. Used
/// by `skill_publish_ability::list_handler` so the curator's
/// `skill.list` verb returns byte-identical rows to what the
/// operator-facing Skills page sees through `fleet.list_abilities`.
/// Single source of truth for the on-disk skill enumeration.
pub(crate) fn list_handler_for_args(args: Value) -> anyhow::Result<Value> {
    list_handler(args)
}

/// `fleet.list_abilities` RPC handler.
///
/// Args: `{ "agent_id": "<name>"? }` — when present, filter to skills
/// owned by that agent; absent or empty = list across every
/// registered agent. Mirrors backend's `ListInstalledReq.AgentID`
/// shape so the backend's call into this ability is a 1:1 forward
/// of its inbound HTTP request params.
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
        .and_then(|o| o.get("agent_id"))
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let registry = agents::load_agents()?;

    // Collect rows in the same shape `easynet skill list --json`
    // emitted historically — the backend already knows how to read
    // that shape (see `cliInstalledRow` in
    // backend/internal/logic/skill/listInstalledLogic.go), and this
    // ability now becomes the definitive source.
    use crate::facade::cli::skill::{
        global_skill_pools_for, scan_global_pool_into, read_install_record, InstallRecord,
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
                            eprintln!(
                                "fleet.list_abilities: skipping {}: {e}",
                                dir_entry.path().display()
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
        .map(|r| serde_json::to_value(r).unwrap_or(Value::Null))
        .collect();

    Ok(json!({ "items": items }))
}

/// JSON Schema for the input. Surfaces in the discovery catalog so
/// a UI knows the verb accepts `{ agent_id: string? }`.
pub fn list_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "agent_id": {
                "type": "string",
                "description": "Restrict the list to skills owned by this agent. Absent = every registered agent."
            }
        },
        "additionalProperties": false,
    })
}

/// Human-readable blurb for discovery surfaces.
pub fn list_description() -> &'static str {
    "List installed skills across registered agents. \
     Combines marketplace installs (<agent-root>/skills/) with the \
     agent-native global pools (~/.claude/skills, ~/.agents/skills)."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_makes_list_dispatchable() {
        // Minimum-viable check: ABILITY_LIST registers an RPC handler
        // that the dispatcher can find. End-to-end (with real agents
        // + skills) is exercised via the smoke script; this test
        // pins the registration shape so a missing register() call
        // trips here instead of in production.
        let mut reg = LocalAbilityRegistry::new();
        register(&mut reg);
        assert!(reg.get_rpc(ABILITY_LIST).is_some());
    }

    #[test]
    fn list_input_schema_is_object_with_optional_agent_id() {
        // The schema is the contract a UI renders. Pin the keys so
        // a typo in `agent_id` (or a future "agent" rename without
        // the backend half) trips here.
        let s = list_input_schema();
        assert_eq!(s["type"], "object");
        assert_eq!(s["additionalProperties"], false);
        let props = s["properties"].as_object().expect("properties is object");
        assert!(props.contains_key("agent_id"));
        assert_eq!(props["agent_id"]["type"], "string");
        // No `required` array — agent_id is optional (omitted = list
        // across every agent).
        assert!(s.get("required").is_none());
    }
}
