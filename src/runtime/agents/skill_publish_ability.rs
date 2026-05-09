// EasyNet CLI — skill.publish + skill.unpublish + skill.list (root meta-abilities)
// =====================================================================================
//
// File: src/runtime/agents/skill_publish_ability.rs
// Description: Root meta-abilities that let a curator session
//              (spawned by `mission.think`) materialise a new skill
//              into a registered agent's skills/ directory, delete
//              an existing one, or list the agent's skills. Sibling
//              of `ability_publish_ability`; the two surfaces are
//              the two sinks the judge picks between when its
//              `value_kind` field is `"ability"` vs `"skill"`.
//
// Why a separate `skill.list` here vs the existing `fleet.list_abilities`?
// -----------------------------------------------------------------------
// `fleet.list_abilities` already returns `InstalledSkill` rows for
// the operator-facing skills page (it is named `list_abilities`
// for legacy reasons; it really lists installed skills). We do
// NOT add a redundant `skill.list` that re-implements that walk;
// instead, this module's `skill.list` is a thin facade that
// delegates to `fleet.list_abilities`'s same on-disk walk so the
// curator's "did I just publish that?" check goes through the
// same source of truth as the Skills page.
//
// Skill on-disk layout (mirrors `easynet skill install`)
// ------------------------------------------------------
//
//   <agent-root>/skills/<skill-name>/
//       SKILL.md               # the curator-authored description
//       .easynet/
//           install.json       # provenance: source = curator:mission.think,
//                              # content_hash, installed_at, size_bytes
//
// The provenance source is `curator:mission.think:<run_id>` (the
// mission run that spawned the curator). This is what
// distinguishes a curator-published skill from a github-installed
// one when an operator audits the skill source later. Backend
// reads `source.kind` already.
//
// What `skill.publish` writes
// ---------------------------
// One file (`SKILL.md`) with the curator's authored body, plus the
// metadata json. Phase 3 does not let the curator ship arbitrary
// additional files — the model is "the skill IS its description".
// Multi-file skills land via `easynet skill install` from a github
// source; that path is unchanged.
//
// Conflict + delete policy
// ------------------------
// Same as `ability.publish`/`ability.unpublish`:
//   * publish refuses to overwrite an existing skill of the same name
//   * unpublish hard-deletes the entire `skills/<name>/` subtree
//   * daemon log captures `[skill.unpublish] owner=… name=…
//     content_hash=…` so the body can be reconstructed from any
//     external backup
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::registry::agents;
use crate::runtime::ability_dispatch::LocalAbilityRegistry;

use crate::runtime::ability_dispatch::OwnerKind;
/// Wire name: `skill.publish`. Matched by curator-issued calls.
pub const ABILITY_PUBLISH: &str = "device.skill.publish";
/// Wire name: `skill.unpublish`. Curator + operator both call it.
pub const ABILITY_UNPUBLISH: &str = "device.skill.unpublish";
/// Wire name: `skill.list`. Thin facade over `fleet.list_abilities`.
pub const ABILITY_LIST: &str = "device.skill.list";

pub fn register(reg: &mut LocalAbilityRegistry) {
    reg.register_rpc_with_owner(
        "device.skill.publish",
        OwnerKind::Device,
        Arc::new(publish_handler),
    );
    reg.register_rpc_with_owner(
        "device.skill.unpublish",
        OwnerKind::Device,
        Arc::new(unpublish_handler),
    );
    reg.register_rpc_with_owner(
        "device.skill.list",
        OwnerKind::Device,
        Arc::new(list_handler),
    );
}

/// `skill.publish` — materialise a curator-authored skill.
///
/// Args:
/// ```json
/// {
///   "owner_agent_id":  "<agent name>",
///   "skill_name":      "<dir-safe slug>",
///   "skill_md":        "<SKILL.md body>",
///   "mission_run_id":  "<curator's mission run id>"   // optional
/// }
/// ```
///
/// Returns:
/// ```json
/// {
///   "ok": true,
///   "owner_agent_id": "<agent name>",
///   "skill_name":     "<slug>",
///   "skill_dir":      "<absolute path to skills/<slug>/>",
///   "content_hash":   "sha256:<hex>"
/// }
/// ```
fn publish_handler(args: Value) -> anyhow::Result<Value> {
    let (owner_id, skill_name, body, run_id) = parse_publish_args(&args)?;
    validate_skill_name(&skill_name)?;
    let (owner_root, agent_type) = resolve_owner_root_and_type(&owner_id)?;

    let skills_dir = skills_dir_for(&owner_root, agent_type);
    let skill_dir = skills_dir.join(&skill_name);
    if skill_dir.exists() {
        anyhow::bail!(
            "skill.publish: skill {skill_name:?} already exists for agent {owner_id:?} \
             at {}; call `skill.unpublish` first to replace it",
            skill_dir.display()
        );
    }

    std::fs::create_dir_all(&skill_dir).map_err(|e| {
        anyhow::anyhow!(
            "skill.publish: failed to create {}: {e}",
            skill_dir.display()
        )
    })?;
    let skill_md_path = skill_dir.join("SKILL.md");
    let hash = content_hash(&body);
    let size_bytes = body.as_bytes().len() as u64;
    crate::persistence::config::atomic_write(&skill_md_path, body.as_bytes())
        .map_err(|e| anyhow::anyhow!("skill.publish: write {}: {e}", skill_md_path.display()))?;

    // Provenance: source.kind = "curator", identifier carries the
    // mission run id that produced this skill so an operator
    // auditing the skill knows which mission.think session
    // authored it.
    let identifier = run_id
        .clone()
        .unwrap_or_else(|| "mission.think".to_string());
    let source = crate::facade::cli::skill::SkillSource {
        kind: "curator".to_string(),
        identifier: identifier.clone(),
        ref_: None,
        subpath: None,
    };
    let installed_at = chrono::Utc::now().to_rfc3339();
    // Strip the `sha256:` prefix for the on-disk install.json —
    // legacy InstallRecord.content_hash carries a bare hex string
    // (matching what `fleet.list_abilities` already emits to the
    // backend / frontend). The wire envelope below keeps the
    // prefix because callers benefit from algorithm tagging.
    let bare_hash = hash.strip_prefix("sha256:").unwrap_or(&hash).to_string();
    let record = crate::facade::cli::skill::InstallRecord {
        name: skill_name.clone(),
        agent_id: owner_id.clone(),
        source,
        skill_tree_hash: bare_hash,
        size_bytes,
        installed_at,
        last_checked_at: None,
        upgrade_available: false,
    };

    let meta_dir = skill_dir.join(".easynet");
    std::fs::create_dir_all(&meta_dir).map_err(|e| {
        anyhow::anyhow!(
            "skill.publish: failed to create {}: {e}",
            meta_dir.display()
        )
    })?;
    let install_path = meta_dir.join("install.json");
    let install_json = serde_json::to_string_pretty(&record)
        .map_err(|e| anyhow::anyhow!("skill.publish: failed to serialise install.json: {e}"))?;
    crate::persistence::config::atomic_write(&install_path, install_json.as_bytes())
        .map_err(|e| anyhow::anyhow!("skill.publish: write {}: {e}", install_path.display()))?;

    eprintln!(
        "[skill.publish] owner={owner_id} name={skill_name} dir={} content_hash={hash}",
        skill_dir.display()
    );

    Ok(json!({
        "ok": true,
        "owner_agent_id": owner_id,
        "skill_name": skill_name,
        "skill_dir": skill_dir.display().to_string(),
        "content_hash": hash,
        "mission_run_id": run_id,
    }))
}

/// `skill.unpublish` — hard-delete a curator-published skill.
///
/// Args:
/// ```json
/// {
///   "owner_agent_id": "<agent name>",
///   "skill_name":     "<slug>"
/// }
/// ```
fn unpublish_handler(args: Value) -> anyhow::Result<Value> {
    let (owner_id, skill_name) = parse_unpublish_args(&args)?;
    validate_skill_name(&skill_name)?;
    let (owner_root, agent_type) = resolve_owner_root_and_type(&owner_id)?;
    let skill_dir = skills_dir_for(&owner_root, agent_type).join(&skill_name);

    if !skill_dir.exists() {
        anyhow::bail!(
            "skill.unpublish: no skill named {skill_name:?} for agent {owner_id:?} \
             (looked at {})",
            skill_dir.display()
        );
    }

    // Capture content hash from the install record before delete so
    // the log line gives operators a recovery handle. If the
    // install.json is missing or unreadable (a hand-edited skill
    // dir), fall back to "unknown" rather than refusing to delete —
    // the operator is asking to remove a directory that exists, the
    // hash is a nice-to-have for the log, not a precondition.
    let install_path = skill_dir.join(".easynet").join("install.json");
    let logged_hash = std::fs::read_to_string(&install_path)
        .ok()
        .and_then(|t| serde_json::from_str::<crate::facade::cli::skill::InstallRecord>(&t).ok())
        .map(|r| format!("sha256:{}", r.skill_tree_hash))
        .unwrap_or_else(|| "unknown".to_string());

    std::fs::remove_dir_all(&skill_dir).map_err(|e| {
        anyhow::anyhow!(
            "skill.unpublish: failed to remove {}: {e}",
            skill_dir.display()
        )
    })?;

    eprintln!(
        "[skill.unpublish] owner={owner_id} name={skill_name} dir={} content_hash={logged_hash}",
        skill_dir.display()
    );

    Ok(json!({
        "ok": true,
        "owner_agent_id": owner_id,
        "skill_name": skill_name,
        "removed_dir": skill_dir.display().to_string(),
        "content_hash": logged_hash,
    }))
}

/// `skill.list` — thin facade over `fleet.list_abilities`.
///
/// Args:
/// ```json
/// { "owner_agent_id": "<agent name>"? }
/// ```
/// (When absent, all agents' skills are listed — same shape
/// `fleet.list_abilities` accepts.)
///
/// Returns: same `{ "items": [InstalledSkill, ...] }` shape as
/// `fleet.list_abilities`. We delegate directly so backend and
/// curator see byte-identical rows.
fn list_handler(args: Value) -> anyhow::Result<Value> {
    // Re-shape: skill.list takes `owner_agent_id`; fleet.list_abilities
    // takes `agent_id` (legacy name). Translate so the curator's
    // ergonomic name passes through to the canonical handler
    // without forcing the curator to remember the legacy field.
    let translated = match args {
        Value::Object(mut map) => {
            if let Some(v) = map.remove("owner_agent_id") {
                map.insert("agent_id".to_string(), v);
            }
            Value::Object(map)
        }
        other => other,
    };
    crate::runtime::agents::skill_ability::list_handler_for_args(translated)
}

// ── helpers ─────────────────────────────────────────────────────────────

fn parse_publish_args(args: &Value) -> anyhow::Result<(String, String, String, Option<String>)> {
    let obj = args
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("skill.publish: args must be a JSON object"))?;
    let owner = obj
        .get("owner_agent_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("skill.publish: missing/empty `owner_agent_id`"))?
        .to_string();
    let name = obj
        .get("skill_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("skill.publish: missing/empty `skill_name`"))?
        .to_string();
    let body = obj
        .get("skill_md")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "skill.publish: missing/empty `skill_md` (string, the SKILL.md body \
                 the skill installs as)"
            )
        })?
        .to_string();
    let run_id = obj
        .get("mission_run_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    Ok((owner, name, body, run_id))
}

fn parse_unpublish_args(args: &Value) -> anyhow::Result<(String, String)> {
    let obj = args
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("skill.unpublish: args must be a JSON object"))?;
    let owner = obj
        .get("owner_agent_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("skill.unpublish: missing/empty `owner_agent_id`"))?
        .to_string();
    let name = obj
        .get("skill_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("skill.unpublish: missing/empty `skill_name`"))?
        .to_string();
    Ok((owner, name))
}

/// Resolve the owner agent's root path AND its runtime type. Needed
/// by the publish path so we can pick the right SKILL.md install
/// location: Claude Code looks for project-local skills under
/// `<cwd>/.claude/skills/<name>/`, not `<cwd>/skills/<name>/`.
/// Codex has no native skill concept; for codex agents we still
/// use `<cwd>/skills/` so EasyNet's own listing surfaces the
/// artifact, but we know the LLM won't auto-load it.
fn resolve_owner_root_and_type(owner_id: &str) -> anyhow::Result<(PathBuf, agents::AgentType)> {
    let registry = agents::load_agents()?;
    let entry = registry.agents.get(owner_id).ok_or_else(|| {
        anyhow::anyhow!(
            "owner_agent_id {owner_id:?} is not registered (registered agents: {:?})",
            registry.agents.keys().collect::<Vec<_>>()
        )
    })?;
    let root = entry
        .root_path
        .clone()
        .unwrap_or_else(|| crate::persistence::config::agents_root().join(owner_id));
    if !root.is_dir() {
        anyhow::bail!(
            "owner agent {owner_id:?} has no on-disk workspace at {}",
            root.display()
        );
    }
    Ok((root, entry.agent_type))
}

/// Pick the on-disk skills directory for a given agent type. This
/// is the LOAD-BEARING piece of skill discovery: Claude Code's
/// skill loader scans `<cwd>/.claude/skills/<name>/SKILL.md`. If
/// EasyNet writes to `<cwd>/skills/<name>/SKILL.md` instead, the
/// skill is published but invisible to the running LLM.
///
/// History (2026-04-29): an earlier version of skill.publish wrote
/// every skill to `<root>/skills/`. The skill artifact existed,
/// `easynet skill list` saw it, `manifests_for` could enumerate
/// the directory — but Claude Code never picked it up because the
/// running `claude` subprocess was looking at
/// `<cwd>/.claude/skills/` per Anthropic's project-local
/// convention. Real e2e (mission.think → curator → skill.publish
/// → restart daemon → agent send) showed `skills_loaded` listing
/// only the EasyNet ability shims, never the freshly-published
/// skill. The fix: route claude-code agents to `.claude/skills/`.
///
/// Codex has no equivalent project-local skill convention; we
/// keep its skills under `<root>/skills/` for EasyNet's audit
/// path, knowing the codex CLI won't auto-load them. A future
/// codex-skill convention (if one ships upstream) would plug in
/// here without changing call sites.
fn skills_dir_for(root: &std::path::Path, agent_type: agents::AgentType) -> PathBuf {
    match agent_type {
        agents::AgentType::ClaudeCode => root.join(".claude").join("skills"),
        agents::AgentType::Codex | agents::AgentType::CodexAppServer => root.join("skills"),
    }
}

/// Refuse skill names that would escape the skills/ directory or
/// trip OS-specific filename restrictions. The allow-list mirrors
/// what existing `easynet skill install` accepts: ASCII alnum plus
/// `-` and `_`. Anything else (path separators, dots, whitespace,
/// non-ASCII) is rejected so a curator's freeform `value_kind`
/// rationale never becomes an attempt to write to
/// `../../something`.
fn validate_skill_name(name: &str) -> anyhow::Result<()> {
    if name.is_empty() {
        anyhow::bail!("skill name must not be empty");
    }
    if name.len() > 100 {
        anyhow::bail!("skill name {} bytes exceeds 100-byte cap", name.len());
    }
    for c in name.chars() {
        let ok = c.is_ascii_alphanumeric() || c == '-' || c == '_';
        if !ok {
            anyhow::bail!(
                "skill name {name:?} contains invalid char {c:?}; allowed: ASCII \
                 alphanumeric plus `-` and `_`"
            );
        }
    }
    Ok(())
}

fn content_hash(body: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(body.as_bytes());
    format!("sha256:{:x}", h.finalize())
}

pub fn publish_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["owner_agent_id", "skill_name", "skill_md"],
        "properties": {
            "owner_agent_id": {"type": "string"},
            "skill_name": {
                "type": "string",
                "description": "Slug used as the directory name. ASCII alnum + `-`/`_` only."
            },
            "skill_md": {
                "type": "string",
                "description": "SKILL.md body. The full text installed as the skill's description."
            },
            "mission_run_id": {
                "type": "string",
                "description": "Optional. The curator's mission run id, recorded as install provenance."
            }
        }
    })
}

pub fn unpublish_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["owner_agent_id", "skill_name"],
        "properties": {
            "owner_agent_id": {"type": "string"},
            "skill_name": {"type": "string"}
        }
    })
}

pub fn list_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "owner_agent_id": {
                "type": "string",
                "description": "Optional. Filter to skills owned by this agent. Absent = all agents."
            }
        }
    })
}

pub fn publish_description() -> &'static str {
    "Publish a curator-authored skill into a registered agent's skills/<name>/ directory. \
     The skill body becomes SKILL.md; provenance is recorded in .easynet/install.json. \
     Refuses to overwrite an existing skill — call skill.unpublish first to replace."
}

pub fn unpublish_description() -> &'static str {
    "Remove a skill from an agent's skills/ directory. Hard delete of the skill subtree; \
     daemon log records the deleted skill's content hash for recovery from backup."
}

pub fn list_description() -> &'static str {
    "List skills installed for an agent (or all agents). Thin facade over \
     fleet.list_abilities; returns the same InstalledSkill row shape."
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::agent_spec::{AgentSpec, RuntimeKind};
    use crate::facade::cli::test_support::HomeGuard;
    use crate::registry::agents::{AgentEntry, AgentRegistry, AgentType};
    use crate::runtime::directory::{AgentDirectory, Location};

    fn materialise_agent(tag: &str, _guard: &HomeGuard) -> String {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let name = format!("test-agent-{tag}-{pid}-{nanos}");
        let agent_root = crate::persistence::config::agents_root().join(&name);
        let spec = AgentSpec::new(&name, RuntimeKind::ClaudeCode);
        let _ = AgentDirectory::create(
            &Location::Local {
                root: agent_root.clone(),
            },
            spec,
        )
        .unwrap();
        let mut registry = agents::load_agents().unwrap_or_else(|_| AgentRegistry::default());
        let mut entry = AgentEntry::new(AgentType::ClaudeCode, None);
        entry.root_path = Some(agent_root.clone());
        registry.agents.insert(name.clone(), entry);
        agents::save_agents(&registry).unwrap();
        name
    }

    #[test]
    fn publish_writes_skill_md_and_install_json() {
        let g = HomeGuard::new();
        let name = materialise_agent("writes", &g);
        let res = publish_handler(json!({
            "owner_agent_id": name,
            "skill_name": "summarise-niche",
            "skill_md": "# Summarise\nA skill the curator wrote.",
            "mission_run_id": "mission-think-001",
        }))
        .expect("publish ok");
        assert_eq!(res["ok"], true);
        let dir = res["skill_dir"].as_str().unwrap();
        let p = std::path::Path::new(dir);
        assert!(p.join("SKILL.md").exists());
        assert!(p.join(".easynet").join("install.json").exists());
        let body = std::fs::read_to_string(p.join(".easynet").join("install.json")).unwrap();
        assert!(body.contains("\"kind\": \"curator\""));
        assert!(body.contains("mission-think-001"));
    }

    #[test]
    fn publish_rejects_overwrite() {
        let g = HomeGuard::new();
        let name = materialise_agent("overwrite", &g);
        publish_handler(json!({
            "owner_agent_id": name,
            "skill_name": "dup",
            "skill_md": "v1",
        }))
        .unwrap();
        let err = publish_handler(json!({
            "owner_agent_id": name,
            "skill_name": "dup",
            "skill_md": "v2",
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("already exists"));
    }

    #[test]
    fn publish_rejects_traversal_in_name() {
        let g = HomeGuard::new();
        let name = materialise_agent("traversal", &g);
        let err = publish_handler(json!({
            "owner_agent_id": name,
            "skill_name": "../escape",
            "skill_md": "x",
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("invalid char"));
    }

    #[test]
    fn unpublish_removes_subtree_and_logs_hash() {
        let g = HomeGuard::new();
        let name = materialise_agent("unpub-removes", &g);
        let pub_res = publish_handler(json!({
            "owner_agent_id": name,
            "skill_name": "to-be-deleted",
            "skill_md": "body",
        }))
        .unwrap();
        let dir = pub_res["skill_dir"].as_str().unwrap().to_string();
        let unpub_res = unpublish_handler(json!({
            "owner_agent_id": name,
            "skill_name": "to-be-deleted",
        }))
        .expect("unpublish ok");
        assert_eq!(unpub_res["ok"], true);
        let h = unpub_res["content_hash"].as_str().unwrap();
        assert!(h.starts_with("sha256:"));
        assert_eq!(h, pub_res["content_hash"].as_str().unwrap());
        assert!(!std::path::Path::new(&dir).exists());
    }

    #[test]
    fn unpublish_errors_when_target_does_not_exist() {
        let g = HomeGuard::new();
        let name = materialise_agent("unpub-missing", &g);
        let err = unpublish_handler(json!({
            "owner_agent_id": name,
            "skill_name": "never-published",
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("no skill named"));
    }

    #[test]
    fn list_handler_returns_published_skill() {
        let g = HomeGuard::new();
        let name = materialise_agent("list", &g);
        publish_handler(json!({
            "owner_agent_id": name,
            "skill_name": "found-me",
            "skill_md": "content",
        }))
        .unwrap();
        let res = list_handler(json!({"owner_agent_id": name})).expect("list ok");
        let items = res["items"].as_array().expect("items array");
        let found = items.iter().any(|item| item["name"] == "found-me");
        assert!(
            found,
            "list_handler must surface the just-published skill: {res}"
        );
    }
}
