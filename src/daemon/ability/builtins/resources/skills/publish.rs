// EasyNet CLI — skill.* root meta-abilities
// =====================================================================================
//
// File: src/daemon/ability/builtins/resources/skills/publish.rs
// Description: Root meta-abilities that let a curator session
//              (spawned by `mission.think`) materialise a new skill
//              into a registered agent runtime's managed skills
//              directory, delete an existing one, list the agent's
//              skills, or inspect the skill package's files. Sibling of
//              `ability_publish_ability`; the two surfaces are the
//              two sinks the judge picks between when its
//              `value_kind` field is `"ability"` vs `"skill"`.
//
// Skill inventory
// ---------------
// `skill.list` is the canonical installed-skill inventory
// surface. Public/network-visible ability descriptors live under
// `meta.list_abilities`; private skill packages live under
// `skill.*`.
//
// Skill on-disk layout (mirrors `easynet skill install`)
// ------------------------------------------------------
//
//   <agent-managed-skills-dir>/<skill-name>/
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
//   * unpublish hard-deletes the entire managed skill subtree
//   * daemon log captures `[skill.unpublish] owner=… name=…
//     content_hash=…` so the body can be reconstructed from any
//     external backup
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::daemon::ability::dispatch::AxonAbilityCatalog;
use crate::daemon::persistence::agent_aggregate::{AgentAggregateRepository, AgentSkillLayout};
use crate::daemon::resources::skills::projection::{
    SkillPublishReceipt, SkillReadFileResponse, SkillTreeEntry, SkillTreeResponse,
    SkillUnpublishReceipt, SkillWriteFileReceipt,
};
use crate::daemon::resources::skills::store::managed_skill_dir_for;

use super::list;
use crate::daemon::ability::dispatch::OwnerKind;
/// Wire name: `skill.publish`. Matched by curator-issued calls.
pub const ABILITY_PUBLISH: &str = crate::daemon::ability::names::resources::SKILL_PUBLISH;
/// Wire name: `skill.unpublish`. Curator + operator both call it.
pub const ABILITY_UNPUBLISH: &str = crate::daemon::ability::names::resources::SKILL_UNPUBLISH;
/// Wire name: `skill.list`.
pub const ABILITY_LIST: &str = crate::daemon::ability::names::resources::SKILL_LIST;
/// Wire name: `skill.tree`. Returns a bounded file tree for a skill package.
pub const ABILITY_TREE: &str = crate::daemon::ability::names::resources::SKILL_TREE;
/// Wire name: `skill.read_file`. Reads one UTF-8 file inside a skill package.
pub const ABILITY_READ_FILE: &str = crate::daemon::ability::names::resources::SKILL_READ_FILE;
/// Wire name: `skill.write_file`. Writes one UTF-8 file inside a skill package.
pub const ABILITY_WRITE_FILE: &str = crate::daemon::ability::names::resources::SKILL_WRITE_FILE;

const MAX_SKILL_FILE_BYTES: u64 = 1024 * 1024;

pub fn register(reg: &mut AxonAbilityCatalog) {
    reg.register_rpc_with_owner(
        "skill.publish",
        OwnerKind::Device,
        Arc::new(publish_handler),
    );
    reg.register_rpc_with_owner(
        "skill.unpublish",
        OwnerKind::Device,
        Arc::new(unpublish_handler),
    );
    reg.register_rpc_with_owner("skill.list", OwnerKind::Device, Arc::new(list::handle));
    reg.register_rpc_with_owner("skill.tree", OwnerKind::Device, Arc::new(tree_handler));
    reg.register_rpc_with_owner(
        "skill.read_file",
        OwnerKind::Device,
        Arc::new(read_file_handler),
    );
    reg.register_rpc_with_owner(
        "skill.write_file",
        OwnerKind::Device,
        Arc::new(write_file_handler),
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
    let (owner_root, layout) = resolve_owner_root_and_layout(&owner_id)?;

    let skills_dir = managed_skill_dir_for(&owner_root, layout);
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
    let size_bytes = body.len() as u64;
    crate::daemon::persistence::config::atomic_write(&skill_md_path, body.as_bytes())
        .map_err(|e| anyhow::anyhow!("skill.publish: write {}: {e}", skill_md_path.display()))?;

    // Provenance: source.kind = "curator", identifier carries the
    // mission run id that produced this skill so an operator
    // auditing the skill knows which mission.think session
    // authored it.
    let identifier = run_id
        .clone()
        .unwrap_or_else(|| "mission.think".to_string());
    let source = crate::daemon::resources::skills::store::SkillSource {
        kind: "curator".to_string(),
        identifier: identifier.clone(),
        ref_: None,
        subpath: None,
    };
    let installed_at = chrono::Utc::now().to_rfc3339();
    // Strip the `sha256:` prefix for the on-disk install.json.
    // The wire envelope below keeps the prefix because callers
    // benefit from algorithm tagging.
    let bare_hash = hash.strip_prefix("sha256:").unwrap_or(&hash).to_string();
    let record = crate::daemon::resources::skills::store::InstallRecord {
        name: skill_name.clone(),
        description: crate::daemon::resources::skills::store::skill_description_from_dir(
            &skill_dir,
        ),
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
    crate::daemon::persistence::config::atomic_write(&install_path, install_json.as_bytes())
        .map_err(|e| anyhow::anyhow!("skill.publish: write {}: {e}", install_path.display()))?;

    let dir_display = format!("{}", skill_dir.display());
    crate::op_event!(
        component = skill_publish,
        kind = skill_published,
        owner = owner_id,
        name = skill_name,
        dir = dir_display,
        content_hash = hash,
    );

    Ok(serde_json::to_value(SkillPublishReceipt::success(
        owner_id,
        skill_name,
        skill_dir.display().to_string(),
        hash,
        run_id,
    ))?)
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
    let (owner_root, layout) = resolve_owner_root_and_layout(&owner_id)?;
    let skill_dir = managed_skill_dir_for(&owner_root, layout).join(&skill_name);

    if !skill_dir.exists() {
        anyhow::bail!(
            "skill.unpublish: no skill named {skill_name:?} for agent {owner_id:?} \
             (looked at {})",
            skill_dir.display()
        );
    }

    // Capture content hash from the install record before delete so
    // the log line gives operators a recovery handle. If the record
    // is absent we log "unknown"; if it is present but malformed, log
    // an explicit marker rather than silently treating non-canonical
    // provenance as a valid install record.
    let install_path = skill_dir.join(".easynet").join("install.json");
    let logged_hash = unpublish_audit_hash(&install_path);

    std::fs::remove_dir_all(&skill_dir).map_err(|e| {
        anyhow::anyhow!(
            "skill.unpublish: failed to remove {}: {e}",
            skill_dir.display()
        )
    })?;

    let dir_display = format!("{}", skill_dir.display());
    crate::op_event!(
        component = skill_unpublish,
        kind = skill_unpublished,
        owner = owner_id,
        name = skill_name,
        dir = dir_display,
        content_hash = logged_hash,
    );

    Ok(serde_json::to_value(SkillUnpublishReceipt::success(
        owner_id,
        skill_name,
        skill_dir.display().to_string(),
        logged_hash,
    ))?)
}

fn unpublish_audit_hash(install_path: &Path) -> String {
    let Ok(body) = std::fs::read_to_string(install_path) else {
        return "unknown".to_string();
    };
    match serde_json::from_str::<crate::daemon::resources::skills::store::InstallRecord>(&body) {
        Ok(record) if record.skill_tree_hash.starts_with("sha256:") => record.skill_tree_hash,
        Ok(record) => format!("sha256:{}", record.skill_tree_hash),
        Err(error) => format!(
            "invalid-install-record:{}",
            install_record_error_category(error.classify())
        ),
    }
}

fn install_record_error_category(category: serde_json::error::Category) -> &'static str {
    match category {
        serde_json::error::Category::Io => "io",
        serde_json::error::Category::Syntax => "syntax",
        serde_json::error::Category::Data => "data",
        serde_json::error::Category::Eof => "eof",
    }
}

/// `skill.tree` — list files inside one installed skill package.
///
/// Args:
/// ```json
/// {
///   "owner_agent_id": "<agent name>",
///   "skill_name": "<slug>",
///   "resource_ura": "easynet:///r/<realm>/resource/agent.<user>.<agent>/skill/<slug>"
/// }
/// ```
fn tree_handler(args: Value) -> anyhow::Result<Value> {
    let (owner_id, skill_name) = parse_owner_skill_args(&args, "skill.tree")?;
    validate_skill_name(&skill_name)?;
    let resource_ura = package_resource_ura_from_args(&args, "skill.tree", &skill_name)?;
    let skill_dir = resolve_readable_skill_dir(&owner_id, &skill_name, "skill.tree")?;
    let mut entries = Vec::new();
    collect_skill_tree_entries(&skill_dir, &skill_dir, &resource_ura, &mut entries)?;
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(serde_json::to_value(SkillTreeResponse::success(
        owner_id,
        skill_name,
        skill_dir.display().to_string(),
        entries,
        resource_ura,
    ))?)
}

/// `skill.read_file` — read a UTF-8 file inside one installed skill package.
fn read_file_handler(args: Value) -> anyhow::Result<Value> {
    let (owner_id, skill_name, rel_path) = parse_skill_file_args(&args, "skill.read_file", false)?;
    validate_skill_name(&skill_name)?;
    let package_ura = package_resource_ura_from_args(&args, "skill.read_file", &skill_name)?;
    let skill_dir = resolve_readable_skill_dir(&owner_id, &skill_name, "skill.read_file")?;
    let rel = validate_skill_relative_path(&rel_path, false)?;
    let full = skill_dir.join(&rel);
    ensure_resolved_inside(&skill_dir, &full, "skill.read_file")?;
    let meta = std::fs::metadata(&full)
        .map_err(|e| anyhow::anyhow!("skill.read_file: metadata {}: {e}", full.display()))?;
    if !meta.is_file() {
        anyhow::bail!("skill.read_file: {} is not a file", rel.display());
    }
    if meta.len() > MAX_SKILL_FILE_BYTES {
        anyhow::bail!(
            "skill.read_file: {} is {} bytes; cap is {} bytes",
            rel.display(),
            meta.len(),
            MAX_SKILL_FILE_BYTES
        );
    }
    let bytes = std::fs::read(&full)
        .map_err(|e| anyhow::anyhow!("skill.read_file: read {}: {e}", full.display()))?;
    let content = String::from_utf8(bytes).map_err(|_| {
        anyhow::anyhow!(
            "skill.read_file: {} is not valid UTF-8; binary skill files are not editable here",
            rel.display()
        )
    })?;
    let rel_wire = rel.to_string_lossy().to_string();
    Ok(serde_json::to_value(SkillReadFileResponse::success(
        owner_id,
        skill_name,
        rel_wire.clone(),
        content,
        meta.len(),
        skill_file_resource_ura(&package_ura, &rel_wire),
    ))?)
}

/// `skill.write_file` — write a UTF-8 file inside one installed skill package.
fn write_file_handler(args: Value) -> anyhow::Result<Value> {
    let (owner_id, skill_name, rel_path) = parse_skill_file_args(&args, "skill.write_file", true)?;
    validate_skill_name(&skill_name)?;
    let package_ura = package_resource_ura_from_args(&args, "skill.write_file", &skill_name)?;
    let obj = args
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("skill.write_file: args must be a JSON object"))?;
    let content = obj
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("skill.write_file: missing `content` string"))?;
    if content.len() as u64 > MAX_SKILL_FILE_BYTES {
        anyhow::bail!(
            "skill.write_file: content is {} bytes; cap is {} bytes",
            content.len(),
            MAX_SKILL_FILE_BYTES
        );
    }

    let skill_dir = resolve_skill_dir(&owner_id, &skill_name, "skill.write_file")?;
    let rel = validate_skill_relative_path(&rel_path, true)?;
    let full = skill_dir.join(&rel);
    ensure_resolved_inside(&skill_dir, &full, "skill.write_file")?;
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            anyhow::anyhow!("skill.write_file: create parent {}: {e}", parent.display())
        })?;
    }
    crate::daemon::persistence::config::atomic_write(&full, content.as_bytes())
        .map_err(|e| anyhow::anyhow!("skill.write_file: write {}: {e}", full.display()))?;
    let hash = refresh_install_record_hash(&skill_dir)?;
    let rel_wire = rel.to_string_lossy().to_string();
    Ok(serde_json::to_value(SkillWriteFileReceipt::success(
        owner_id,
        skill_name,
        rel_wire.clone(),
        content.len() as u64,
        hash,
        skill_file_resource_ura(&package_ura, &rel_wire),
    ))?)
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

fn package_resource_ura_from_args(
    args: &Value,
    verb: &str,
    skill_name: &str,
) -> anyhow::Result<String> {
    let ura = args
        .as_object()
        .and_then(|obj| obj.get("resource_ura"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("{verb}: missing/empty `resource_ura`"))?;

    let parsed = crate::core::ura::parse_ura(ura)
        .map_err(|e| anyhow::anyhow!("{verb}: invalid resource_ura {ura:?}: {e}"))?;
    if parsed.kind != crate::core::ura::URAKind::Resource {
        anyhow::bail!(
            "{verb}: resource_ura must be a resource URA, got {}",
            parsed.kind
        );
    }
    let owner_id = parsed
        .resource_owner_id()
        .ok_or_else(|| anyhow::anyhow!("{verb}: resource_ura is missing resource owner"))?;
    if !owner_id.starts_with("agent.") {
        anyhow::bail!("{verb}: resource_ura must identify an agent skill package, got {ura:?}");
    }
    let expected_path = format!("skill/{skill_name}");
    let resource_path = parsed.resource_path().unwrap_or_default();
    if resource_path.trim_end_matches('/') != expected_path {
        anyhow::bail!(
            "{verb}: resource_ura path must be {expected_path:?}, got {:?}",
            resource_path
        );
    }
    Ok(ura.trim_end_matches('/').to_string())
}

fn skill_file_resource_ura(package_ura: &str, rel_path: &str) -> String {
    let base = package_ura.trim().trim_end_matches('/');
    let clean = rel_path.trim_start_matches('/');
    if clean.is_empty() {
        return base.to_string();
    }
    let Ok(parsed) = crate::core::ura::parse_ura(base) else {
        return base.to_string();
    };
    if parsed.kind != crate::core::ura::URAKind::Resource {
        return base.to_string();
    }
    let resource_path = parsed.resource_path().unwrap_or_default();
    let child_path = if resource_path.is_empty() {
        format!("file/{clean}")
    } else {
        format!("{}/file/{clean}", resource_path.trim_end_matches('/'))
    };
    let Some(owner_id) = parsed.resource_owner_id() else {
        return base.to_string();
    };
    crate::core::ura::resource_dot_ura(&parsed.realm, owner_id, &child_path)
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

fn parse_owner_skill_args(args: &Value, verb: &str) -> anyhow::Result<(String, String)> {
    let obj = args
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("{verb}: args must be a JSON object"))?;
    let owner = obj
        .get("owner_agent_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("{verb}: missing/empty `owner_agent_id`"))?
        .to_string();
    let name = obj
        .get("skill_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("{verb}: missing/empty `skill_name`"))?
        .to_string();
    Ok((owner, name))
}

fn parse_skill_file_args(
    args: &Value,
    verb: &str,
    allow_create: bool,
) -> anyhow::Result<(String, String, String)> {
    let (owner, name) = parse_owner_skill_args(args, verb)?;
    let obj = args
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("{verb}: args must be a JSON object"))?;
    let path = obj
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("{verb}: missing/empty `path`"))?
        .to_string();
    if !allow_create && path.ends_with('/') {
        anyhow::bail!("{verb}: `path` must point to a file, got directory-like path {path:?}");
    }
    Ok((owner, name, path))
}

/// Resolve the owner agent's root path and skill layout. Directory
/// projection is delegated to the daemon skill store so publish, list,
/// install, upgrade, and remove cannot drift by runtime.
fn resolve_owner_root_and_layout(owner_id: &str) -> anyhow::Result<(PathBuf, AgentSkillLayout)> {
    let owner =
        AgentAggregateRepository::load_registered_agent_workspace(owner_id, "skill.publish")?;
    let root = owner.root_path().to_path_buf();
    if !root.is_dir() {
        anyhow::bail!(
            "owner agent {owner_id:?} has no on-disk workspace at {}",
            root.display()
        );
    }
    Ok((root, owner.skill_layout()))
}

fn skill_dir_candidates_for(
    root: &Path,
    layout: AgentSkillLayout,
    skill_name: &str,
) -> anyhow::Result<Vec<PathBuf>> {
    let mut candidates = vec![managed_skill_dir_for(root, layout).join(skill_name)];
    if let Some(global_dir) =
        crate::daemon::resources::skills::store::global_skill_dir_for(layout, skill_name)?
    {
        if !candidates.iter().any(|candidate| candidate == &global_dir) {
            candidates.push(global_dir);
        }
    }
    Ok(candidates)
}

fn resolve_skill_dir(owner_id: &str, skill_name: &str, verb: &str) -> anyhow::Result<PathBuf> {
    let (owner_root, layout) = resolve_owner_root_and_layout(owner_id)?;
    for candidate in skill_dir_candidates_for(&owner_root, layout, skill_name)? {
        if candidate.is_dir() {
            return Ok(candidate);
        }
    }
    anyhow::bail!(
        "{verb}: no skill named {skill_name:?} for agent {owner_id:?} under {}",
        owner_root.display()
    )
}

fn resolve_readable_skill_dir(
    owner_id: &str,
    skill_name: &str,
    verb: &str,
) -> anyhow::Result<PathBuf> {
    if let Some(global_pool) =
        crate::daemon::resources::skills::store::GlobalSkillPoolRef::parse_owner_id(owner_id, verb)?
    {
        if let Some(path) = global_pool.skill_dir(skill_name)? {
            return Ok(path);
        }
        anyhow::bail!(
            "{verb}: no global skill named {skill_name:?} in pool {:?}",
            global_pool.label()
        );
    }
    resolve_skill_dir(owner_id, skill_name, verb)
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

fn validate_skill_relative_path(path: &str, allow_new_file: bool) -> anyhow::Result<PathBuf> {
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        anyhow::bail!("skill file path must be relative, got {path:?}");
    }
    let mut clean = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!("skill file path {path:?} escapes the skill directory")
            }
        }
    }
    if clean.as_os_str().is_empty() {
        anyhow::bail!("skill file path must not be empty");
    }
    if clean.components().next().and_then(|c| match c {
        Component::Normal(s) => Some(s == ".easynet"),
        _ => None,
    }) == Some(true)
    {
        anyhow::bail!("skill file path {path:?} targets EasyNet metadata, not skill source");
    }
    if !allow_new_file && clean.to_string_lossy().ends_with('/') {
        anyhow::bail!("skill file path must point to a file");
    }
    Ok(clean)
}

fn ensure_resolved_inside(root: &Path, path: &Path, verb: &str) -> anyhow::Result<()> {
    let root_canon = root
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("{verb}: canonicalize {}: {e}", root.display()))?;
    let parent = if path.exists() {
        path
    } else {
        path.parent()
            .ok_or_else(|| anyhow::anyhow!("{verb}: path has no parent: {}", path.display()))?
    };
    let parent_canon = parent
        .canonicalize()
        .or_else(|_| {
            if let Some(existing) = parent.ancestors().find(|p| p.exists()) {
                existing.canonicalize()
            } else {
                parent.canonicalize()
            }
        })
        .map_err(|e| anyhow::anyhow!("{verb}: canonicalize {}: {e}", parent.display()))?;
    if !parent_canon.starts_with(&root_canon) {
        anyhow::bail!(
            "{verb}: {} resolves outside skill directory {}",
            path.display(),
            root.display()
        );
    }
    Ok(())
}

fn collect_skill_tree_entries(
    root: &Path,
    dir: &Path,
    package_ura: &str,
    out: &mut Vec<SkillTreeEntry>,
) -> anyhow::Result<()> {
    let mut children: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| anyhow::anyhow!("skill.tree: read_dir {}: {e}", dir.display()))?
        .flatten()
        .collect();
    children.sort_by_key(|entry| entry.path());
    for entry in children {
        let path = entry.path();
        let rel = path.strip_prefix(root).unwrap_or(&path);
        if rel.components().next().and_then(|c| match c {
            Component::Normal(s) => Some(s == ".easynet"),
            _ => None,
        }) == Some(true)
        {
            continue;
        }
        let rel_str = rel.to_string_lossy();
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(e) => {
                let path_display = format!("{}", path.display());
                let err_msg = format!("{e}");
                crate::op_event!(
                    component = skill_tree,
                    kind = entry_skipped,
                    level = "warn",
                    path = path_display,
                    error = err_msg,
                );
                continue;
            }
        };
        if meta.is_dir() {
            out.push(SkillTreeEntry::directory(
                rel_str.to_string(),
                skill_file_resource_ura(package_ura, &rel_str),
            ));
            collect_skill_tree_entries(root, &path, package_ura, out)?;
        } else if meta.is_file() {
            out.push(SkillTreeEntry::file(
                rel_str.to_string(),
                meta.len(),
                skill_file_resource_ura(package_ura, &rel_str),
            ));
        }
    }
    Ok(())
}

fn refresh_install_record_hash(skill_dir: &Path) -> anyhow::Result<String> {
    let hash = hash_skill_tree(skill_dir)?;
    let record_path = skill_dir.join(".easynet").join("install.json");
    if record_path.exists() {
        let mut record =
            crate::daemon::resources::skills::store::read_install_record(&record_path)?;
        record.skill_tree_hash = hash.clone();
        record.size_bytes = skill_tree_size_bytes(skill_dir)?;
        let body = serde_json::to_string_pretty(&record)
            .map_err(|e| anyhow::anyhow!("skill.write_file: serialise install.json: {e}"))?;
        crate::daemon::persistence::config::atomic_write(&record_path, body.as_bytes()).map_err(
            |e| anyhow::anyhow!("skill.write_file: write {}: {e}", record_path.display()),
        )?;
    }
    Ok(hash)
}

fn hash_skill_tree(root: &Path) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};
    let mut files = Vec::new();
    collect_hash_files(root, root, &mut files)?;
    files.sort();
    let mut h = Sha256::new();
    for rel in files {
        h.update(rel.to_string_lossy().as_bytes());
        h.update([0u8]);
        let full = root.join(&rel);
        let bytes = std::fs::read(&full)?;
        h.update((bytes.len() as u64).to_be_bytes());
        h.update(bytes);
    }
    Ok(format!("sha256:{:x}", h.finalize()))
}

fn collect_hash_files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let rel = path.strip_prefix(root).unwrap_or(&path);
        if rel.components().next().and_then(|c| match c {
            Component::Normal(s) => Some(s == ".easynet"),
            _ => None,
        }) == Some(true)
        {
            continue;
        }
        let meta = entry.metadata()?;
        if meta.is_dir() {
            collect_hash_files(root, &path, out)?;
        } else if meta.is_file() {
            out.push(rel.to_path_buf());
        }
    }
    Ok(())
}

fn skill_tree_size_bytes(root: &Path) -> anyhow::Result<u64> {
    let mut files = Vec::new();
    collect_hash_files(root, root, &mut files)?;
    let mut total = 0;
    for rel in files {
        total += std::fs::metadata(root.join(rel))?.len();
    }
    Ok(total)
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
            },
            "agent_ura": {
                "type": "string",
                "description": "Canonical owner Agent URA. Filters to that hosted agent and derives authoritative skill resource_ura values."
            },
            "subject_ura": {
                "type": "string",
                "description": "Owner Agent URA or skill package Resource URA. A skill Resource URA filters to that single skill."
            }
        },
        "additionalProperties": false
    })
}

pub fn tree_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["owner_agent_id", "skill_name", "resource_ura"],
        "properties": {
            "owner_agent_id": {"type": "string"},
            "skill_name": {"type": "string"},
            "resource_ura": {
                "type": "string",
                "description": "Canonical skill package resource URA returned by skill.list."
            }
        },
        "additionalProperties": false
    })
}

pub fn read_file_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["owner_agent_id", "skill_name", "resource_ura", "path"],
        "properties": {
            "owner_agent_id": {"type": "string"},
            "skill_name": {"type": "string"},
            "resource_ura": {
                "type": "string",
                "description": "Canonical skill package resource URA returned by skill.list."
            },
            "path": {
                "type": "string",
                "description": "Relative file path inside the skill directory, for example SKILL.md."
            }
        },
        "additionalProperties": false
    })
}

pub fn write_file_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["owner_agent_id", "skill_name", "resource_ura", "path", "content"],
        "properties": {
            "owner_agent_id": {"type": "string"},
            "skill_name": {"type": "string"},
            "resource_ura": {
                "type": "string",
                "description": "Canonical skill package resource URA returned by skill.list."
            },
            "path": {
                "type": "string",
                "description": "Relative file path inside the skill directory, for example SKILL.md."
            },
            "content": {
                "type": "string",
                "description": "UTF-8 file content. Maximum 1 MiB."
            }
        },
        "additionalProperties": false
    })
}

pub fn publish_description() -> &'static str {
    "Publish a curator-authored skill into a registered Agent runtime's managed skills directory. \
     The skill body becomes SKILL.md; provenance is recorded in .easynet/install.json. \
     Refuses to overwrite an existing skill — call skill.unpublish first to replace."
}

pub fn unpublish_description() -> &'static str {
    "Remove a skill from a registered Agent runtime's managed skills directory. \
     Hard delete of the skill subtree; \
     daemon log records the deleted skill's content hash for recovery from backup."
}

pub fn list_description() -> &'static str {
    "List skills installed for an agent (or all agents). Canonical skill inventory \
     surface; returns InstalledSkill rows."
}

pub fn tree_description() -> &'static str {
    "List the file tree for one installed skill package. The lookup is constrained to \
     the selected agent's visible skill directories and returns relative paths plus file sizes."
}

pub fn read_file_description() -> &'static str {
    "Read one UTF-8 file from an installed skill package. The path is relative to the \
     skill directory and traversal outside that directory is rejected."
}

pub fn write_file_description() -> &'static str {
    "Write one UTF-8 file inside an installed skill package. The path is relative to \
     the skill directory, traversal is rejected, and install metadata is rehashed."
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::commands::test_support::HomeGuard;
    use crate::core::agent::spec::{AgentSpec, RuntimeKind};
    use crate::daemon::execution::mission::directory::{AgentDirectory, Location};
    use crate::daemon::persistence::agent_registry as agents;
    use crate::daemon::persistence::agent_registry::{AgentEntry, AgentRegistry, AgentType};

    fn materialise_agent(tag: &str, _guard: &HomeGuard) -> String {
        materialise_agent_with_runtime(tag, RuntimeKind::ClaudeCode, AgentType::ClaudeCode)
    }

    fn materialise_agent_with_runtime(
        tag: &str,
        runtime: RuntimeKind,
        agent_type: AgentType,
    ) -> String {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let name = format!("test-agent-{tag}-{pid}-{nanos}");
        let agent_root = crate::daemon::persistence::config::agents_root().join(&name);
        let spec = AgentSpec::new(&name, runtime);
        let _ = AgentDirectory::create(
            &Location::Local {
                root: agent_root.clone(),
            },
            spec,
        )
        .unwrap();
        let mut registry = agents::load_agents().unwrap_or_else(|_| AgentRegistry::default());
        let mut entry = AgentEntry::new(agent_type, None);
        entry.root_path = Some(agent_root.clone());
        registry.agents.insert(format!("default/{name}"), entry);
        agents::save_agents(&registry).unwrap();
        name
    }

    fn persist_hosted_agent_ura(name: &str) -> String {
        let agent_ura = crate::core::ura::agent_ura("localhost", "dev", name);
        let mut local = crate::daemon::persistence::local_agents::load().unwrap_or_else(|_| {
            crate::daemon::persistence::local_agents::LocalAgentsFile::default()
        });
        local.host_device_agent_ura = "easynet:///r/localhost/device/dev-1".to_string();
        crate::daemon::persistence::local_agents::upsert_hosted_agent(
            &mut local, "llm", name, &agent_ura,
        );
        crate::daemon::persistence::local_agents::save(&local).unwrap();
        agent_ura
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
    fn publish_writes_codex_skill_to_runtime_project_dir() {
        let _g = HomeGuard::new();
        let name =
            materialise_agent_with_runtime("codex-writes", RuntimeKind::Codex, AgentType::Codex);
        let res = publish_handler(json!({
            "owner_agent_id": name,
            "skill_name": "codex-visible",
            "skill_md": "# Codex Visible",
        }))
        .expect("publish ok");
        let skill_dir = PathBuf::from(res["skill_dir"].as_str().unwrap());
        assert!(
            skill_dir.ends_with(".agents/skills/codex-visible"),
            "codex skills must land in the runtime project skill root: {}",
            skill_dir.display()
        );
        assert!(skill_dir.join("SKILL.md").exists());
        assert!(
            !crate::daemon::persistence::config::agents_root()
                .join(&name)
                .join("skills")
                .join("codex-visible")
                .exists(),
            "codex publish must not leave a retired root-level managed skill"
        );
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
    fn unpublish_marks_malformed_install_record_without_accepting_legacy_fields() {
        let g = HomeGuard::new();
        let name = materialise_agent("unpub-malformed-install", &g);
        let pub_res = publish_handler(json!({
            "owner_agent_id": name,
            "skill_name": "bad-provenance",
            "skill_md": "body",
        }))
        .unwrap();
        let dir = PathBuf::from(pub_res["skill_dir"].as_str().unwrap());
        std::fs::write(
            dir.join(".easynet").join("install.json"),
            r#"{
                "name": "bad-provenance",
                "agent_id": "agent",
                "source": {"kind": "curator", "identifier": "mission"},
                "content_hash": "sha256:deadbeef",
                "size_bytes": 1,
                "installed_at": "2026-04-23T00:00:00Z",
                "upgrade_available": false,
                "legacy_content_hash": "sha256:legacy"
            }"#,
        )
        .unwrap();

        let unpub_res = unpublish_handler(json!({
            "owner_agent_id": name,
            "skill_name": "bad-provenance",
        }))
        .expect("unpublish still removes an existing directory");
        let hash = unpub_res["content_hash"].as_str().unwrap();
        assert!(
            hash.starts_with("invalid-install-record:"),
            "malformed provenance must be explicit in audit output: {hash}"
        );
        assert!(!dir.exists());
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
        let res = list::handle(json!({"owner_agent_id": name})).expect("list ok");
        let items = res["items"].as_array().expect("items array");
        let found = items.iter().any(|item| item["name"] == "found-me");
        assert!(
            found,
            "list_handler must surface the just-published skill: {res}"
        );
        let row = items
            .iter()
            .find(|item| item["name"] == "found-me")
            .unwrap();
        assert_eq!(row["description"], "content");
    }

    #[test]
    fn list_handler_filters_by_agent_ura_and_subject_resource() {
        let g = HomeGuard::new();
        let first = materialise_agent("list-scope-a", &g);
        let second = materialise_agent("list-scope-b", &g);
        let first_ura = persist_hosted_agent_ura(&first);
        let second_ura = persist_hosted_agent_ura(&second);
        publish_handler(json!({
            "owner_agent_id": first,
            "skill_name": "first-skill",
            "skill_md": "first",
        }))
        .unwrap();
        publish_handler(json!({
            "owner_agent_id": second,
            "skill_name": "second-skill",
            "skill_md": "second",
        }))
        .unwrap();

        let by_agent = list::handle(json!({ "agent_ura": first_ura })).expect("list by agent_ura");
        let items = by_agent["items"].as_array().unwrap();
        assert_eq!(
            items.len(),
            1,
            "agent_ura must scope to one owner: {by_agent}"
        );
        assert_eq!(items[0]["agent_id"], first);
        assert_eq!(items[0]["name"], "first-skill");
        assert_eq!(
            items[0]["resource_ura"],
            crate::core::ura::resource_dot_ura(
                "localhost",
                &format!("agent.dev.{first}"),
                "skill/first-skill"
            )
        );

        let skill_subject = crate::core::ura::resource_dot_ura(
            "localhost",
            &format!("agent.dev.{first}"),
            "skill/first-skill",
        );
        let by_subject =
            list::handle(json!({ "subject_ura": skill_subject })).expect("list by subject_ura");
        let items = by_subject["items"].as_array().unwrap();
        assert_eq!(
            items.len(),
            1,
            "skill resource subject must scope to one skill"
        );
        assert_eq!(items[0]["name"], "first-skill");

        let err = list::handle(json!({
            "agent_ura": second_ura,
            "subject_ura": crate::core::ura::resource_dot_ura(
                "localhost",
                &format!("agent.dev.{first}"),
                "skill/first-skill",
            )
        }))
        .unwrap_err()
        .to_string();
        assert!(err.contains("must match"), "got {err}");
    }

    #[test]
    fn skill_file_resource_ura_extends_resource_via_axon_builder() {
        let package_ura =
            "easynet:///r/localhost/resource/agent.dev.frontend-engineer/skill/alive-video";
        assert_eq!(
            skill_file_resource_ura(package_ura, "/docs/SKILL.md"),
            "easynet:///r/localhost/resource/agent.dev.frontend-engineer/skill/alive-video/file/docs/SKILL.md",
        );
    }

    #[test]
    fn tree_and_read_file_are_scoped_to_skill_dir() {
        let g = HomeGuard::new();
        let name = materialise_agent("tree-read", &g);
        let resource_ura = test_skill_resource_ura(&name, "inspectable");
        let published = publish_handler(json!({
            "owner_agent_id": name,
            "skill_name": "inspectable",
            "skill_md": "# Inspectable\nBody",
        }))
        .unwrap();
        let dir = PathBuf::from(published["skill_dir"].as_str().unwrap());
        let notes_dir = dir.join("notes");
        std::fs::create_dir_all(&notes_dir).unwrap();
        crate::daemon::persistence::config::atomic_write(&notes_dir.join("guide.md"), b"guide")
            .unwrap();

        let tree = tree_handler(json!({
            "owner_agent_id": name,
            "skill_name": "inspectable",
            "resource_ura": resource_ura,
        }))
        .expect("tree ok");
        let files = tree["files"].as_array().unwrap();
        assert!(files
            .iter()
            .any(|f| f["path"] == "SKILL.md" && f["type"] == "file"));
        assert!(files
            .iter()
            .any(|f| f["path"] == "notes/guide.md" && f["type"] == "file"));
        assert!(!files.iter().any(|f| f["path"] == ".easynet/install.json"));

        let read = read_file_handler(json!({
            "owner_agent_id": name,
            "skill_name": "inspectable",
            "resource_ura": resource_ura,
            "path": "notes/guide.md",
        }))
        .expect("read ok");
        assert_eq!(read["content"], "guide");
    }

    #[test]
    fn tree_and_read_file_resolve_global_pool_skill_returned_by_list() {
        let g = HomeGuard::new();
        let name = materialise_agent("global-tree-read", &g);
        let skill_dir = crate::daemon::persistence::config::home_dir()
            .join(".claude")
            .join("skills")
            .join("alive-video");
        std::fs::create_dir_all(&skill_dir).unwrap();
        crate::daemon::persistence::config::atomic_write(
            &skill_dir.join("SKILL.md"),
            b"---\nname: alive-video\ndescription: Alive Video\n---\n# Alive Video\n",
        )
        .unwrap();
        crate::daemon::persistence::config::atomic_write(
            &skill_dir.join("guide.md"),
            b"global guide",
        )
        .unwrap();

        let listed = list::handle(json!({"owner_agent_id": name})).expect("list ok");
        assert!(
            listed["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["name"] == "alive-video"),
            "list must surface the global skill before tree/read can address it: {listed}"
        );
        let row = listed["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == "alive-video")
            .unwrap();
        assert_eq!(row["description"], "Alive Video");

        let resource_ura = test_skill_resource_ura(&name, "alive-video");
        let tree = tree_handler(json!({
            "owner_agent_id": name,
            "skill_name": "alive-video",
            "resource_ura": resource_ura,
        }))
        .expect("tree resolves global skill");
        assert!(tree["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["path"] == "guide.md" && f["type"] == "file"));

        let read = read_file_handler(json!({
            "owner_agent_id": name,
            "skill_name": "alive-video",
            "resource_ura": resource_ura,
            "path": "guide.md",
        }))
        .expect("read resolves global skill");
        assert_eq!(read["content"], "global guide");
    }

    #[test]
    fn tree_and_read_file_accept_unscoped_global_pool_owner() {
        let _g = HomeGuard::new();
        let skill_dir = crate::daemon::persistence::config::home_dir()
            .join(".claude")
            .join("skills")
            .join("global-inspectable");
        std::fs::create_dir_all(&skill_dir).unwrap();
        crate::daemon::persistence::config::atomic_write(
            &skill_dir.join("SKILL.md"),
            b"---\nname: global-inspectable\ndescription: Global Inspectable\n---\n",
        )
        .unwrap();
        crate::daemon::persistence::config::atomic_write(
            &skill_dir.join("guide.md"),
            b"global guide",
        )
        .unwrap();

        let tree = tree_handler(json!({
            "owner_agent_id": "global:claude-global",
            "skill_name": "global-inspectable",
            "resource_ura": "easynet:///r/localhost/resource/agent.dev.claude-global/skill/global-inspectable",
        }))
        .expect("tree resolves unscoped global owner");
        assert_eq!(tree["owner_agent_id"], "global:claude-global");
        assert!(tree["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["path"] == "guide.md" && f["type"] == "file"));

        let read = read_file_handler(json!({
            "owner_agent_id": "global:claude-global",
            "skill_name": "global-inspectable",
            "resource_ura": "easynet:///r/localhost/resource/agent.dev.claude-global/skill/global-inspectable",
            "path": "guide.md",
        }))
        .expect("read resolves unscoped global owner");
        assert_eq!(read["content"], "global guide");
    }

    #[test]
    fn write_file_updates_content_and_rejects_traversal() {
        let g = HomeGuard::new();
        let name = materialise_agent("write", &g);
        let resource_ura = test_skill_resource_ura(&name, "editable");
        let published = publish_handler(json!({
            "owner_agent_id": name,
            "skill_name": "editable",
            "skill_md": "old",
        }))
        .unwrap();
        let before_hash = published["content_hash"].as_str().unwrap().to_string();

        let write = write_file_handler(json!({
            "owner_agent_id": name,
            "skill_name": "editable",
            "resource_ura": resource_ura,
            "path": "SKILL.md",
            "content": "new body",
        }))
        .expect("write ok");
        assert_eq!(write["ok"], true);
        assert_ne!(write["content_hash"].as_str().unwrap(), before_hash);

        let read = read_file_handler(json!({
            "owner_agent_id": name,
            "skill_name": "editable",
            "resource_ura": resource_ura,
            "path": "SKILL.md",
        }))
        .expect("read ok");
        assert_eq!(read["content"], "new body");

        let err = write_file_handler(json!({
            "owner_agent_id": name,
            "skill_name": "editable",
            "resource_ura": resource_ura,
            "path": "../escape.md",
            "content": "nope",
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("escapes"));

        let err = write_file_handler(json!({
            "owner_agent_id": name,
            "skill_name": "editable",
            "resource_ura": resource_ura,
            "path": ".easynet/install.json",
            "content": "{}",
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("metadata"));
    }

    fn test_skill_resource_ura(owner: &str, skill: &str) -> String {
        crate::core::ura::resource_dot_ura(
            "localhost",
            &format!("agent.dev.{owner}"),
            &format!("skill/{skill}"),
        )
    }
}
