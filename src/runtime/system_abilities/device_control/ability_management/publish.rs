// EasyNet CLI — ability.publish + ability.unpublish (root meta-abilities)
// =========================================================================
//
// File: src/runtime/system_abilities/device_control/ability_management/publish.rs
// Description: Root abilities that let an agent (typically a curator
//              session spawned by `mission.think`) materialise a new
//              ability into a registered agent's `abilities/` dir, or
//              delete an existing one.
//
// Why these are root, not per-agent
// ---------------------------------
// A curator session is spawned ad-hoc by `mission.think`. It does not
// have its own workspace or a stable `<agent>.publish` route — the
// session that fires `ability.publish` is short-lived and may not be
// the same agent that owns the resulting manifest. The owner is
// passed in `args.owner_ura`; the daemon resolves the on-disk
// root from the URA's agent id and the local agent registry.
//
// Trust model (Phase 2 baseline)
// ------------------------------
// Today's daemon serves only the local control.sock — already
// gated by unix permissions to the local user. There is no inbound
// channel that could forge a `caller_agent_id`; consequently this
// ability does not enforce a `caller == owner` check. When a
// future RFC introduces a cross-device publish channel
// (`federation.publish_ability` or similar), publisher signature
// verification through the RFC-002 keyring layer plugs in here.
// The right place to add it is `validate_authorisation()` below;
// the rest of the function (file write, conflict check, log line)
// does not need to change.
//
// Ontology
// --------
// `ability.publish` corresponds to the "general / shared / result-
// oriented" sink in the judge's `value_kind = "ability"` branch:
// the experience is broad enough that every agent on the device
// benefits. The other sink (`value_kind = "skill"`) is the agent's
// own skill pool — that is `skill.publish`, in a sibling module.
//
// What `ability.publish` writes
// -----------------------------
// `<owner-root>/abilities/<verb>.ability.toml`. The on-disk shape
// is the same one `easynet agent abilities` reads; `manifests_for`
// already enumerates this directory so the freshly-published
// ability is visible to the next `discover` without any extra
// notification. The file stem (`<verb>`) MUST match the manifest's
// `name` field — `list_ability_manifests` rejects a divergence
// loud, so we double-check at write time to fail fast at publish
// time rather than on the next discover scan.
//
// Conflict policy
// ---------------
// If `<verb>.ability.toml` already exists for the owner, publish is
// rejected. Overwriting an existing manifest would let a curator
// session silently shadow the agent's own author-pinned ability —
// an attack surface that can never be intended. The caller can
// `ability.unpublish` first if they truly want to replace.
//
// `ability.unpublish` semantics
// -----------------------------
// Hard delete. The request selects the target by canonical
// `ability_ura`; the handler derives the owner-local manifest stem
// and local agent id from that single identity, then removes the
// on-disk file. The daemon log captures
// `[ability.unpublish] owner=<id> name=<verb> content_hash=<sha256>`
// so an operator who needs to recover the manifest can grep the log
// and reconstruct. We considered a
// soft-delete (visibility → selfish + tombstone) and rejected it:
// EAL has no static dependency graph, so we cannot offer a
// "pre-delete impact report" anyway, and two-state deletes
// (visible / invisible-but-still-callable-by-owner) just bloat
// discover semantics. Hard delete + log is the simplest model
// that does not lose information.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::core::ability_spec::AbilityManifest;
use crate::registry::agents;
use crate::runtime::ability_dispatch::AxonAbilityCatalog;
use crate::runtime::ability_dispatch::OwnerKind;
use crate::runtime::directory::ABILITY_MANIFEST_SUFFIX;

/// Wire name of the publish meta-ability. Pinned because the
/// curator session in `mission.think` calls it by string; a rename
/// breaks every published mission.
pub const ABILITY_PUBLISH: &str = crate::daemon::ability::names::federation::ABILITY_PUBLISH;

/// Wire name of the unpublish meta-ability. Same pinning rationale
/// as `ABILITY_PUBLISH`.
pub const ABILITY_UNPUBLISH: &str = crate::daemon::ability::names::federation::ABILITY_UNPUBLISH;

/// Register both verbs on the registry. Stateless: the handlers
/// reach disk directly and read the agent registry on every call,
/// because publish is rare and the registry lookup is cheap (no
/// hot-path concern). No captured state to keep coherent.
pub fn register(reg: &mut AxonAbilityCatalog) {
    reg.register_rpc_with_owner(
        "ability.publish",
        OwnerKind::Device,
        Arc::new(publish_handler),
    );
    reg.register_rpc_with_owner(
        "ability.unpublish",
        OwnerKind::Device,
        Arc::new(unpublish_handler),
    );
}

/// `ability.publish` handler.
///
/// Args:
/// ```json
/// {
///   "owner_ura": "easynet:///r/<realm>/agent/<user>.<agent>",
///   "manifest_toml":  "<full ability.toml text>"
/// }
/// ```
///
/// Returns:
/// ```json
/// {
///   "ok": true,
///   "owner_ura": "easynet:///r/<realm>/agent/<user>.<agent>",
///   "public_name":    "<verb>",
///   "ability_ura":    "easynet:///r/<realm>/ability/<user>.<agent>.<verb>",
///   "path":           "<absolute path to written manifest>",
///   "content_hash":   "sha256:<hex>"
/// }
/// ```
///
/// Errors with a clear message when:
///   * `owner_ura` is missing, empty, malformed, or its agent
///     id is not registered locally
///   * `manifest_toml` is missing, empty, or fails `from_toml_str`
///     validation (bad schema, empty argv, unknown sandbox profile,
///     etc.)
///   * the manifest's `name` is reserved (`chat`) — author-pinned
///     manifests on every agent must not be shadowed by a publish
///   * a manifest with the same name already exists (refuse
///     overwrite — see module doc)
fn publish_handler(args: Value) -> anyhow::Result<Value> {
    let (owner_ura, owner_id, manifest_toml) = parse_publish_args(&args)?;
    let owner_root = resolve_owner_root(&owner_id)?;
    validate_authorisation(&owner_id)?;

    let manifest = AbilityManifest::from_toml_str(&manifest_toml)?;
    let verb = manifest.name().to_string();
    reject_reserved_verb(&verb)?;
    let ability_ura = crate::ura::owner_ability_ura(&owner_ura, &verb)
        .ok_or_else(|| anyhow::anyhow!("ability.publish: cannot derive ability_ura"))?;

    let abilities_dir = owner_root.join("abilities");
    std::fs::create_dir_all(&abilities_dir).map_err(|e| {
        anyhow::anyhow!(
            "ability.publish: failed to create abilities dir {}: {e}",
            abilities_dir.display()
        )
    })?;

    let target = abilities_dir.join(format!("{verb}{ABILITY_MANIFEST_SUFFIX}"));
    if target.exists() {
        anyhow::bail!(
            "ability.publish: an ability named {verb:?} already exists for agent {owner_id:?} \
             at {}; call `ability.unpublish` first to replace it",
            target.display()
        );
    }

    // Write through the canonical manifest serialiser so we always
    // stamp the current schema_version + normalised field order,
    // not the caller-provided text. This prevents a malformed-but-
    // parseable input (e.g. a manifest whose author hand-typed a
    // stale `schema_version`) from polluting the on-disk surface.
    let body = manifest.to_toml_string()?;
    let hash = content_hash(&body);
    crate::persistence::config::atomic_write(&target, body.as_bytes()).map_err(|e| {
        anyhow::anyhow!("ability.publish: failed to write {}: {e}", target.display())
    })?;

    let path_display = format!("{}", target.display());
    crate::op_event!(
        component = ability_publish,
        kind = ability_published,
        owner = owner_id,
        name = verb,
        path = path_display,
        content_hash = hash,
    );

    Ok(json!({
        "ok": true,
        "owner_ura": owner_ura,
        "public_name": verb,
        "ability_ura": ability_ura,
        "path": target.display().to_string(),
        "content_hash": hash,
    }))
}

/// `ability.unpublish` handler.
///
/// Args:
/// ```json
/// {
///   "ability_ura":    "easynet:///r/<realm>/ability/<user>.<agent>.<verb>"
/// }
/// ```
///
/// Returns:
/// ```json
/// {
///   "ok": true,
///   "owner_ura": "easynet:///r/<realm>/agent/<user>.<agent>",
///   "public_name":    "<verb>",
///   "ability_ura":    "easynet:///r/<realm>/ability/<user>.<agent>.<verb>",
///   "removed_path":   "<absolute path of the file that was deleted>",
///   "content_hash":   "sha256:<hex of the body that was deleted>"
/// }
/// ```
///
/// Errors with a clear message when:
///   * `ability_ura` is missing/empty, malformed, not agent-owned,
///     or its agent id is not registered locally
///   * the named ability does not exist on disk for that owner
///   * the verb is reserved (`chat`) — refuse to delete the
///     baseline that every agent ships with; an operator who truly
///     wants to remove it must do so manually
fn unpublish_handler(args: Value) -> anyhow::Result<Value> {
    let (owner_ura, owner_id, ability_ura, verb) = parse_unpublish_args(&args)?;
    let owner_root = resolve_owner_root(&owner_id)?;
    validate_authorisation(&owner_id)?;
    reject_reserved_verb(&verb)?;

    let abilities_dir = owner_root.join("abilities");
    let target = abilities_dir.join(format!("{verb}{ABILITY_MANIFEST_SUFFIX}"));
    if !target.exists() {
        anyhow::bail!(
            "ability.unpublish: no ability named {verb:?} found for agent {owner_id:?} \
             (looked at {})",
            target.display()
        );
    }

    // Read first so we can record the deleted body's hash in the
    // log line. An operator who needs to recover the manifest after
    // an unpublish has the hash + the daemon log timestamp; the
    // pair points at the relevant entry in any external backup.
    let body = std::fs::read_to_string(&target).map_err(|e| {
        anyhow::anyhow!(
            "ability.unpublish: failed to read {} before delete: {e}",
            target.display()
        )
    })?;
    let hash = content_hash(&body);

    std::fs::remove_file(&target).map_err(|e| {
        anyhow::anyhow!(
            "ability.unpublish: failed to remove {}: {e}",
            target.display()
        )
    })?;

    let path_display = format!("{}", target.display());
    crate::op_event!(
        component = ability_unpublish,
        kind = ability_unpublished,
        owner = owner_id,
        name = verb,
        path = path_display,
        content_hash = hash,
    );

    Ok(json!({
        "ok": true,
        "owner_ura": owner_ura,
        "public_name": verb,
        "ability_ura": ability_ura,
        "removed_path": target.display().to_string(),
        "content_hash": hash,
    }))
}

// ── helpers ─────────────────────────────────────────────────────────────

fn parse_publish_args(args: &Value) -> anyhow::Result<(String, String, String)> {
    let obj = args
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("ability.publish: args must be a JSON object"))?;
    let owner_ura = obj
        .get("owner_ura")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "ability.publish: missing/empty `owner_ura` (canonical agent URA, \
                 whose abilities/ dir will receive the manifest)"
            )
        })?;
    let owner_id = agent_id_from_owner_ura("ability.publish", owner_ura)?;
    let manifest = obj
        .get("manifest_toml")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "ability.publish: missing/empty `manifest_toml` (string, the full \
                 ability.toml text — schema_version, name, description, \
                 input_schema, optional [exec])"
            )
        })?
        .to_string();
    Ok((owner_ura.to_string(), owner_id, manifest))
}

fn parse_unpublish_args(args: &Value) -> anyhow::Result<(String, String, String, String)> {
    let obj = args
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("ability.unpublish: args must be a JSON object"))?;
    let ability_ura = obj
        .get("ability_ura")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "ability.unpublish: missing/empty `ability_ura` (canonical Ability URA \
                 for the agent-owned ability to delete)"
            )
        })?
        .to_string();
    let (owner_ura, owner_id, public_name) =
        agent_owner_and_public_name_from_ability_ura(&ability_ura)?;
    Ok((owner_ura, owner_id, ability_ura, public_name))
}

fn resolve_owner_root(owner_id: &str) -> anyhow::Result<PathBuf> {
    let registry = agents::load_agents()?;
    let entry = registry.agents.get(owner_id).ok_or_else(|| {
        anyhow::anyhow!(
            "owner agent id {owner_id:?} is not registered (registered agents: {:?}); \
             create the agent first via `easynet agent new`",
            registry.agents.keys().collect::<Vec<_>>()
        )
    })?;
    let root = entry
        .root_path
        .clone()
        .unwrap_or_else(|| crate::persistence::config::agents_root().join(owner_id));
    if !root.is_dir() {
        anyhow::bail!(
            "owner agent {owner_id:?} has no on-disk workspace at {} — cannot publish \
             into a non-materialised agent",
            root.display()
        );
    }
    Ok(root)
}

fn agent_owner_and_public_name_from_ability_ura(
    ability_ura: &str,
) -> anyhow::Result<(String, String, String)> {
    let parsed = crate::ura::parse_ura(ability_ura)
        .map_err(|e| anyhow::anyhow!("ability.unpublish: invalid `ability_ura`: {e}"))?;
    if parsed.kind != crate::ura::URAKind::Ability {
        anyhow::bail!("ability.unpublish: `ability_ura` must be an Ability URA");
    }
    let ability = parsed
        .ability()
        .ok_or_else(|| anyhow::anyhow!("ability.unpublish: `ability_ura` has no ability tail"))?;
    let (user_id, agent_id) = match ability.owner {
        crate::ura::AbilityOwner::Agent { user_id, agent_id } => (user_id, agent_id),
        other => {
            anyhow::bail!("ability.unpublish: ability_ura must be agent-owned, got {other:?}");
        }
    };
    let public_name = crate::ura::ability_name_from_parts(&parsed).ok_or_else(|| {
        anyhow::anyhow!("ability.unpublish: ability_ura `{ability_ura}` has no public ability name")
    })?;
    let owner_ura = crate::ura::agent_ura(&parsed.realm, &user_id, &agent_id);
    Ok((owner_ura, agent_id, public_name))
}

fn agent_id_from_owner_ura(context: &str, owner_ura: &str) -> anyhow::Result<String> {
    let parsed = crate::ura::parse_ura(owner_ura)
        .map_err(|e| anyhow::anyhow!("{context}: invalid `owner_ura`: {e}"))?;
    if parsed.kind != crate::ura::URAKind::Agent {
        anyhow::bail!("{context}: `owner_ura` must be an Agent URA");
    }
    // DEC-F048: publish/unpublish is a hosted user-agent development
    // surface; a device-sponsored System Agent declares only its
    // device-native abilities and cannot publish here (RFC-005 §3.1.2).
    if parsed.device_agent_ids().is_some() {
        anyhow::bail!(
            "{context}: `owner_ura` is a device-sponsored System Agent \
             (RFC-005 §3.1.2, DEC-F048); publish/unpublish is a hosted \
             user-agent surface"
        );
    }
    let Some((_, agent_id)) = parsed.agent_ids() else {
        anyhow::bail!("{context}: `owner_ura` is missing agent_id");
    };
    Ok(agent_id.to_string())
}

/// Phase 2 baseline: trust the local control.sock's user-level
/// gating. There is no `caller_agent_id` in the envelope today so we
/// cannot enforce caller==owner; see module preamble for why this
/// is acceptable and where to plug keyring verification when a
/// cross-device publish channel lands.
fn validate_authorisation(_owner_id: &str) -> anyhow::Result<()> {
    Ok(())
}

/// Reserved verbs that must never be the target of publish/unpublish.
/// `chat` is the universal default ability every agent ships with;
/// letting curator publish or delete it would corrupt the agent's
/// baseline contract. The set is small on purpose — adding more is
/// cheap, removing one is breaking.
fn reject_reserved_verb(verb: &str) -> anyhow::Result<()> {
    const RESERVED: &[&str] = &["chat"];
    if RESERVED.contains(&verb) {
        anyhow::bail!(
            "verb {verb:?} is reserved and cannot be published or unpublished via \
             ability.publish/unpublish; it is the agent's baseline ability"
        );
    }
    Ok(())
}

/// SHA-256 of the manifest body, prefixed with `sha256:` so the log
/// line and the JSON envelope both name the algorithm explicitly. A
/// future migration to a different hash function (blake3 etc.)
/// changes the prefix; readers parse the prefix and fail loud on
/// unknown algorithms instead of silently misinterpreting a hex
/// string.
fn content_hash(body: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(body.as_bytes());
    format!("sha256:{:x}", h.finalize())
}

pub fn publish_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["owner_ura", "manifest_toml"],
        "properties": {
            "owner_ura": {
                "type": "string",
                "description": "Canonical Agent URA whose abilities/ dir receives the manifest. Its agent id must be registered locally."
            },
            "manifest_toml": {
                "type": "string",
                "description": "Full ability.toml body. The file stem on disk is taken from the manifest's `name` field."
            }
        }
    })
}

pub fn unpublish_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["ability_ura"],
        "properties": {
            "ability_ura": {
                "type": "string",
                "description": "Canonical agent-owned Ability URA to delete. Its agent id must be registered locally and its public name must NOT be reserved (e.g. `chat`)."
            }
        }
    })
}

pub fn publish_description() -> &'static str {
    "Publish a new ability into a registered agent's abilities/ directory. The ability \
     becomes visible to the next discover scan. Refuses to overwrite an existing ability; \
     call ability.unpublish first to replace."
}

pub fn unpublish_description() -> &'static str {
    "Remove an ability from an agent's abilities/ directory. Hard delete; the daemon log \
     records the deleted manifest's content hash so the body can be recovered from a \
     backup if needed. Reserved verbs (e.g. `chat`) cannot be deleted via this surface."
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::agent_spec::{AgentSpec, RuntimeKind};

    #[test]
    fn owner_ura_resolution_dual_shape() {
        // User-owned agent resolves to its agent_id.
        assert_eq!(
            agent_id_from_owner_ura("ability.publish", "easynet:///r/localhost/agent/dev.claude")
                .unwrap(),
            "claude"
        );
        // Device-sponsored System Agent is refused with the
        // normative citation (DEC-F048; F-047 verdict v2).
        let err = agent_id_from_owner_ura(
            "ability.publish",
            "easynet:///r/localhost/agent/device.dev-1.terminal",
        )
        .expect_err("System Agent cannot own the publish surface");
        let msg = err.to_string();
        assert!(msg.contains("RFC-005 §3.1.2"), "{msg}");
        assert!(msg.contains("device-sponsored System Agent"), "{msg}");
    }
    use crate::cli::test_support::HomeGuard;
    use crate::registry::agents::{AgentEntry, AgentRegistry, AgentType};
    use crate::runtime::directory::{AgentDirectory, Location};

    /// Materialise a throwaway agent inside a `HomeGuard`-isolated
    /// `~/.easynet/`. The HomeGuard already holds the process-global
    /// home_lock, so callers do not need their own mutex; passing
    /// the guard back to the caller keeps it alive for the test
    /// body and restores HOME on drop.
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

    fn well_formed_manifest_toml(verb: &str) -> String {
        format!(
            r#"schema_version = "1"
name = "{verb}"
description = "test ability"
[input_schema]
type = "object"
"#
        )
    }

    fn owner_ura(agent_id: &str) -> String {
        crate::ura::agent_ura("test-realm", "alice", agent_id)
    }

    fn ability_ura(agent_id: &str, public_name: &str) -> String {
        crate::ura::ability_ura("test-realm", "alice", agent_id, public_name)
    }

    #[test]
    fn publish_writes_manifest_under_owner_abilities_dir() {
        let g = HomeGuard::new();
        let name = materialise_agent("publish-writes", &g);
        let toml = well_formed_manifest_toml("summarise");
        let owner_ura = owner_ura(&name);
        let res = publish_handler(json!({
            "owner_ura": owner_ura,
            "manifest_toml": toml,
        }))
        .expect("publish ok");
        assert_eq!(res["ok"], true);
        assert_eq!(res["public_name"], "summarise");
        assert_eq!(res["ability_ura"], ability_ura(&name, "summarise"));
        let path = res["path"].as_str().unwrap();
        assert!(
            std::path::Path::new(path).exists(),
            "manifest file must exist on disk: {path}"
        );
        // Discoverable via list_ability_manifests too.
        let entry = agents::load_agents()
            .unwrap()
            .agents
            .get(&name)
            .cloned()
            .unwrap();
        let manifests = crate::runtime::agent_ability_specs::manifests_for(&name, &entry);
        assert!(
            manifests.iter().any(|m| m.name() == "summarise"),
            "published ability must show up in manifests_for"
        );
    }

    #[test]
    fn publish_rejects_overwrite() {
        let g = HomeGuard::new();
        let name = materialise_agent("publish-overwrite", &g);
        let toml = well_formed_manifest_toml("foo");
        publish_handler(json!({
            "owner_ura": owner_ura(&name),
            "manifest_toml": toml.clone(),
        }))
        .unwrap();
        let err = publish_handler(json!({
            "owner_ura": owner_ura(&name),
            "manifest_toml": toml,
        }))
        .unwrap_err();
        assert!(
            format!("{err}").contains("already exists"),
            "second publish must reject: {err}"
        );
    }

    #[test]
    fn publish_rejects_reserved_chat_verb() {
        let g = HomeGuard::new();
        let name = materialise_agent("publish-reserved", &g);
        let toml = well_formed_manifest_toml("chat");
        let err = publish_handler(json!({
            "owner_ura": owner_ura(&name),
            "manifest_toml": toml,
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("reserved"));
    }

    #[test]
    fn publish_rejects_unknown_owner() {
        let g = HomeGuard::new();
        let _real = materialise_agent("publish-unknown", &g);
        let toml = well_formed_manifest_toml("foo");
        let err = publish_handler(json!({
            "owner_ura": owner_ura("no-such-agent-xyz"),
            "manifest_toml": toml,
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("not registered"));
    }

    #[test]
    fn publish_rejects_malformed_manifest_toml() {
        let g = HomeGuard::new();
        let name = materialise_agent("publish-malformed", &g);
        let err = publish_handler(json!({
            "owner_ura": owner_ura(&name),
            "manifest_toml": "this is not toml{{{",
        }))
        .unwrap_err();
        assert!(
            format!("{err}").contains("parse") || format!("{err}").contains("toml"),
            "expected toml parse error: {err}"
        );
    }

    #[test]
    fn unpublish_removes_published_manifest_and_logs_hash() {
        let g = HomeGuard::new();
        let name = materialise_agent("unpublish-removes", &g);
        let toml = well_formed_manifest_toml("bar");
        let pub_res = publish_handler(json!({
            "owner_ura": owner_ura(&name),
            "manifest_toml": toml,
        }))
        .unwrap();
        let path = pub_res["path"].as_str().unwrap().to_string();

        let unpub_res = unpublish_handler(json!({
            "ability_ura": ability_ura(&name, "bar"),
        }))
        .expect("unpublish ok");
        assert_eq!(unpub_res["ok"], true);
        assert_eq!(unpub_res["public_name"], "bar");
        assert_eq!(unpub_res["ability_ura"], ability_ura(&name, "bar"));
        let returned_hash = unpub_res["content_hash"].as_str().unwrap();
        assert!(returned_hash.starts_with("sha256:"));
        assert_eq!(returned_hash, pub_res["content_hash"].as_str().unwrap());
        assert!(
            !std::path::Path::new(&path).exists(),
            "file must be deleted after unpublish: {path}"
        );
    }

    #[test]
    fn unpublish_errors_when_target_does_not_exist() {
        let g = HomeGuard::new();
        let name = materialise_agent("unpublish-missing", &g);
        let err = unpublish_handler(json!({
            "ability_ura": ability_ura(&name, "never-published"),
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("no ability named"));
    }

    #[test]
    fn unpublish_rejects_reserved_chat() {
        let g = HomeGuard::new();
        let name = materialise_agent("unpublish-reserved", &g);
        let err = unpublish_handler(json!({
            "ability_ura": ability_ura(&name, "chat"),
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("reserved"));
    }
}
