// EasyNet CLI — Runtime Abilities (default-chat surface)
// =============================================================
//
// File: src/daemon/execution/mission/agent_ability_specs.rs
// Description: Enumerates the abilities that a locally-registered
//              agent (claude-code, codex, codex-app-server) exposes
//              to the EasyNet federation as remotely-callable tools.
//
// Terminology
// -----------
// An agent is a Tier-2 first-class network entity
// (`easynet://agents/<owner>/<name>`). Agents are *not* abilities.
// What this module enumerates is the set of *abilities each
// agent publishes*; today that set is exactly one — `chat`,
// formed from the agent's default input channel. Future PRs
// may grow it (`voice`, `exec`, etc.) by pushing new specs
// into the returned `Vec`. When you see "agent's default input
// as a chat ability" in comments around here, that is the
// target concept; read any lingering "agent-as-ability" as a
// historical shorthand that was imprecise.
//
// Why this module exists
// ----------------------
// A locally-installed AI agent (Claude Code, Codex, ...) is a live
// subprocess binding on this node, not a distributable capability
// package. Its committed descriptor is registered in the daemon's
// canonical control-plane catalogue, bound to daemon Invocation, and
// published from the same live catalogue snapshot used by discovery.
//
// This module is the neutral ground between the registry
// (`~/.easynet/agents.json`, which records what the operator has
// installed locally), discovery labels, publication prelude code,
// and the MCP profile projection
// (`daemon::ability::catalog::profiles::mcp`). It answers
// exactly one question — "for agent X of type Y, what abilities does
// it offer as network-visible tools, and what is the JSON schema of
// each ability's arguments" — and answers it the same way from all
// call sites:
//
//   1. `federation::read_model::a2a_labels::build` — to include in
//      `a2a.agents_json[*].skills`
//      so federated peers *discover* the abilities without calling
//      anything.
//   2. `daemon::ability::catalog::profiles::mcp` — to project
//      those same abilities into MCP tool descriptors and route each
//      tool call back into daemon Invocation.
//   3. daemon publication/invocation paths — to advertise and invoke
//      hosted-agent abilities without inventing a second ability list.
//
// Keeping both paths fed from one function is the load-bearing
// property: discovery and dispatch cannot drift apart, which
// would otherwise be the single most confusing bug category
// ("agent shows up in the UI but returns 'unknown tool' on
// invocation", or the inverse).
//
// Return type is `Vec<AgentAbilitySpec>` (not `Option<_>` or a fixed
// struct) so adding a second ability per agent — say `voice` or
// `exec` — is purely additive. The current set is a single `chat`
// ability; the `Vec` shape makes the room for expansion explicit at
// the type level rather than inviting a later "I'll just add a
// field" refactor that would break every caller at once.
//
// Naming
// ------
// Ability names use the form `<agent>.<verb>` (e.g. `claude.chat`).
// The `.` is the separator and is explicitly reserved from agent
// names by `registry::agents::validate_agent_name` (which rejects
// `agent.name`), so a collision between an ability name and a
// network tool name ("invoke_ability", "list_devices", …) is
// structurally impossible — pinned by `name_shape_cannot_collide_with_network_tools`
// in the tests below.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

#[cfg(test)]
use serde_json::json;
use serde_json::Value;

use crate::daemon::execution::mission::directory::AgentDirectory;
use crate::daemon::persistence::agent_registry::AgentEntry;

/// One ability exposed by a locally-installed agent.
///
/// The fields are the intersection of what every consumer of this
/// discovery/hint spec needs:
///
///   * `name`      — the tool name as it will appear on the MCP wire
///                   and in `a2a.agents_json[*].skills[*].name`. Must
///                   obey the `<agent>.<verb>` shape and be unique
///                   within a single agent's ability list.
///   * `description` — one-sentence human-readable blurb, surfaced
///                   by agent-side tool pickers when the agent is
///                   choosing which tool to call. Not a protocol
///                   field; may be tuned for readability.
///
/// `AgentAbilitySpec::new(...)` still accepts the manifest input
/// schema and validates that it is a JSON object, but the schema is
/// intentionally not retained here. `AbilityManifest` and descriptor
/// projection own schema transport; this type is only the
/// discovery/prompt DTO.
#[derive(Debug, Clone)]
pub struct AgentAbilitySpec {
    name: String,
    description: String,
}

impl AgentAbilitySpec {
    /// Construct a spec, validating the fields that carry invariants
    /// the consumers of this module rely on.
    ///
    /// Rejects:
    ///   - empty / whitespace-only names (the MCP wire cannot name a
    ///     tool that has no name),
    ///   - names without an `<agent>.<verb>` shape (collision
    ///     prevention, see module docstring),
    ///   - `parameters` whose top level is not a JSON object
    ///     (OpenAI / Axon both expect `{"type": "object", ...}`).
    ///
    /// Callers that build specs programmatically should prefer the
    /// helpers at the module level (`abilities_for` et al.) rather
    /// than hand-rolling — those helpers have been reviewed against
    /// the invariants once, so a new caller gets them for free.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: Value,
    ) -> Result<Self, &'static str> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err("ability name must not be empty");
        }
        if !name.contains('.') {
            // Agent names are `[a-z0-9_-]+` (see `validate_agent_name`),
            // so a dot-less ability name cannot originate from one of
            // our own generators. Reject at construction so a later
            // "let me hand-craft a spec" call site fails loud.
            return Err("ability name must use the `<agent>.<verb>` shape");
        }
        if !parameters.is_object() {
            return Err("ability parameters must be a JSON object (JSON Schema)");
        }
        Ok(Self {
            name,
            description: description.into(),
        })
    }

    /// The network-visible tool name. Stable; the identity under which
    /// this ability is both advertised (in `a2a.agents_json`) and
    /// dispatched (through the daemon's committed catalogue binding).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Human-readable description. Safe to mutate across releases;
    /// not a protocol field.
    pub fn description(&self) -> &str {
        &self.description
    }
}

/// Build the ability list for one agent entry.
///
/// Source of truth: the `<agent-root>/abilities/*.ability.toml`
/// files on disk. This function:
///
/// 1. Resolves the agent's root from `entry.root_path`.
/// 2. Opens the root as an `AgentDirectory`; `agent.toml` is the
///    required source of truth.
/// 3. Reads every manifest in the abilities directory and converts
///    each to an `AgentAbilitySpec` whose qualified name is
///    `<agent>.<verb>`. The manifest is the protocol-level source
///    for `name`, `description`, and `input_schema`.
///
/// Invalid rows fail closed by returning no network-visible abilities.
/// The daemon must not invent a callable `<agent>.chat` from a fat
/// registry row: ability identity is owned by manifests under the
/// AgentDirectory.
pub fn abilities_for(agent_name: &str, entry: &AgentEntry) -> Vec<AgentAbilitySpec> {
    abilities_from_manifests(agent_name, entry)
}

/// Like `abilities_for`, but returns the full `AbilityManifest` for
/// each ability rather than the discovery-trimmed `AgentAbilitySpec`.
///
/// The dispatch path needs more than the discovery shape: when a
/// manifest pins an executor binding (`[exec]`) the handler must see
/// the executor config to route the call to the correct runtime. A
/// manifest without `[exec]` is discoverable metadata, not an invocable
/// route. `abilities_for` strips the executor field on its way to the
/// wire-level spec; this helper keeps it.
///
/// Returns one entry per on-disk `<verb>.ability.toml` in the
/// agent's root, in the same order as `abilities_for`. Invalid
/// registry rows return an empty vector; callers should treat that as
/// "this agent publishes no manifest-backed abilities".
pub fn manifests_for(
    agent_name: &str,
    entry: &AgentEntry,
) -> Vec<crate::daemon::ability::manifest::AbilityManifest> {
    (*manifests_for_shared(agent_name, entry)).clone()
}

// ── Manifest catalog snapshot ───────────────────────────────────────
//
// Catalog reads are snapshot reads. Every network-visible list of an
// agent's abilities used to re-open the AgentDirectory and re-parse
// every `<verb>.ability.toml` on each call — O(agents × manifests)
// disk IO + TOML parsing per discover, plus one full re-parse on
// every dispatch (invoke and chat both resolve manifests). The cache
// below keys each agent root by a stat-only signature (file name,
// length, mtime of `agent.toml` + `*.ability.toml`), so the hot path
// costs one `read_dir` and zero parsing. Any publish / edit / delete
// changes the signature and rebuilds that one agent's entry. Entries
// for roots that leave the registry linger until process exit —
// bounded by the number of distinct roots ever served.

type SharedManifests = std::sync::Arc<Vec<crate::daemon::ability::manifest::AbilityManifest>>;

#[derive(PartialEq, Eq)]
struct ManifestDirSignature(Vec<(std::ffi::OsString, u64, Option<std::time::SystemTime>)>);

/// Stat-only signature of the files [`AgentDirectory`] reads when
/// listing manifests: `<root>/agent.toml` (carries the spec name)
/// plus every `<root>/abilities/*.ability.toml`. A missing
/// `abilities/` dir is a valid "nothing declared" state; an
/// unreadable root returns `None` (fail closed, uncached).
fn manifest_dir_signature(root: &std::path::Path) -> Option<ManifestDirSignature> {
    if !root.is_dir() {
        return None;
    }
    let mut signature = Vec::new();
    if let Ok(meta) = std::fs::metadata(root.join("agent.toml")) {
        signature.push(("agent.toml".into(), meta.len(), meta.modified().ok()));
    }
    if let Ok(entries) = std::fs::read_dir(root.join("abilities")) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if !name
                .to_string_lossy()
                .ends_with(crate::daemon::execution::mission::directory::ABILITY_MANIFEST_SUFFIX)
            {
                continue;
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            signature.push((name, meta.len(), meta.modified().ok()));
        }
    }
    signature.sort();
    Some(ManifestDirSignature(signature))
}

/// One cached agent root. `spec_name` is the directory's OWN
/// `agent.toml` name — requests are validated against it at serve
/// time, so a mis-keyed registry row (name ≠ directory) yields the
/// same fail-closed empty list as before without evicting the
/// rightful owner's snapshot.
struct CachedAgentManifests {
    spec_name: String,
    signature: ManifestDirSignature,
    manifests: SharedManifests,
}

fn manifest_cache(
) -> &'static std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, CachedAgentManifests>>
{
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, CachedAgentManifests>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Shared-snapshot variant of [`manifests_for`]: returns the cached
/// `Arc` without cloning manifest contents. Hot paths (dispatch,
/// discover) should prefer this; the owned-`Vec` wrapper exists for
/// callers that mutate or consume the list.
pub fn manifests_for_shared(agent_name: &str, entry: &AgentEntry) -> SharedManifests {
    let Some(surface_name) = project_agent_surface_name(agent_name) else {
        eprintln!(
            "manifests_for[{agent_name}]: invalid Agent identifier for ability surface projection"
        );
        return SharedManifests::default();
    };
    let Some(root) = entry.root_path.as_ref() else {
        eprintln!(
            "manifests_for[{agent_name}]: registry row is missing root_path; publishing no abilities"
        );
        return SharedManifests::default();
    };
    let Some(signature) = manifest_dir_signature(root) else {
        eprintln!(
            "manifests_for[{agent_name}]: root_path {} is not a readable directory; publishing no abilities",
            root.display()
        );
        return SharedManifests::default();
    };

    let cached = {
        let cache = manifest_cache().lock().unwrap_or_else(|p| p.into_inner());
        cache.get(root.as_path()).and_then(|c| {
            (c.signature == signature)
                .then(|| (c.spec_name.clone(), std::sync::Arc::clone(&c.manifests)))
        })
    };
    let (spec_name, manifests) = match cached {
        Some(hit) => hit,
        None => {
            // Slow path: open + parse OUTSIDE the lock so one agent's
            // rebuild never serializes every other reader. Concurrent
            // rebuilds of the same root are benign — identical data,
            // last insert wins.
            let dir = match AgentDirectory::open(root) {
                Ok(dir) => dir,
                Err(e) => {
                    eprintln!(
                        "manifests_for[{agent_name}]: failed to open agent directory at {}: {e}; publishing no abilities",
                        root.display()
                    );
                    return SharedManifests::default();
                }
            };
            let manifests: SharedManifests = match dir.list_ability_manifests() {
                Ok(m) => std::sync::Arc::new(m),
                Err(e) => {
                    eprintln!(
                        "manifests_for[{agent_name}]: failed to enumerate ability manifests: {e}"
                    );
                    SharedManifests::default()
                }
            };
            let spec_name = dir.spec().name.clone();
            let mut cache = manifest_cache().lock().unwrap_or_else(|p| p.into_inner());
            cache.insert(
                root.clone(),
                CachedAgentManifests {
                    spec_name: spec_name.clone(),
                    signature,
                    manifests: std::sync::Arc::clone(&manifests),
                },
            );
            (spec_name, manifests)
        }
    };

    if spec_name != surface_name {
        eprintln!(
            "manifests_for[{agent_name}]: root_path {} belongs to agent {spec_name:?}; publishing no abilities",
            root.display()
        );
        return SharedManifests::default();
    }
    manifests
}

/// Read the agent's on-disk ability manifests and convert them to
/// network-visible specs. Returns an empty list when there is no usable
/// `root_path`, when the directory is unreadable, or when no manifests
/// are declared.
///
/// Open/enumeration failures are logged and converted to an empty list,
/// never panic. Discovery failing closed (no abilities) is preferable to
/// discovery failing loud in a long-lived daemon where one bad manifest
/// should not take the whole roster offline; operator-facing CLI paths keep
/// fail-loud semantics.
fn abilities_from_manifests(agent_name: &str, entry: &AgentEntry) -> Vec<AgentAbilitySpec> {
    let manifests = manifests_for_shared(agent_name, entry);
    let Some(surface_name) = project_agent_surface_name(agent_name) else {
        return Vec::new();
    };
    let mut specs = Vec::with_capacity(manifests.len());
    for manifest in manifests.iter() {
        match AgentAbilitySpec::new(
            manifest.qualified_name(&surface_name),
            manifest.description().to_string(),
            manifest.input_schema().clone(),
        ) {
            Ok(spec) => specs.push(spec),
            Err(e) => {
                eprintln!(
                    "abilities_for[{agent_name}]: dropping malformed manifest {:?}: {e}",
                    manifest.name()
                );
            }
        }
    }
    specs
}

fn project_agent_surface_name(agent_identifier: &str) -> Option<String> {
    if agent_identifier.contains('/') {
        let agent_id = crate::core::agent::id::AgentId::parse(agent_identifier).ok()?;
        if agent_id.to_string() != agent_identifier {
            return None;
        }
        return Some(agent_id.name);
    }
    crate::core::agent::id::AgentId::new(crate::core::agent::id::DEFAULT_TENANT, agent_identifier)
        .ok()
        .map(|agent| agent.name)
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! Tests aim to guard the invariants the module's docstrings
    //! promise — not just "it builds a spec". Each test names the
    //! invariant in its function name so a failure report tells the
    //! operator which promise just broke.

    use super::*;
    use crate::core::agent::spec::AgentSpec;
    use crate::core::agent::spec::RuntimeKind;
    use crate::daemon::ability::manifest::AbilityManifest;
    use crate::daemon::execution::mission::directory::{
        AgentDirectory, Location, ABILITY_MANIFEST_SUFFIX,
    };
    use std::sync::atomic::{AtomicU64, Ordering};

    fn entry_named(name: &str, t: RuntimeKind) -> AgentEntry {
        make_agent_on_disk(name, t).1
    }

    #[test]
    fn abilities_for_returns_empty_when_root_path_missing() {
        use crate::cli::commands::test_support::HomeGuard;
        let _g = HomeGuard::new();

        let entry = AgentEntry::new(RuntimeKind::ClaudeCode, None);
        assert!(entry.root_path.is_none());
        let abilities = abilities_for("ghost-agent", &entry);
        assert!(
            abilities.is_empty(),
            "missing root_path must not synthesize ghost abilities: {abilities:?}"
        );
    }

    #[test]
    fn abilities_for_returns_exactly_one_chat_spec_per_agent_type() {
        // The current contract is "one ability per agent, named
        // `<agent>.chat`". Pinning the count here is what catches a
        // future "let me silently add a second ability" patch that
        // would break downstream consumers which assume a one-to-one
        // agent↔ability mapping.
        for t in [
            RuntimeKind::ClaudeCode,
            RuntimeKind::Codex,
            RuntimeKind::CodexAppServer,
        ] {
            let entry = entry_named("test-agent", t);
            let abilities = abilities_for("test-agent", &entry);
            assert_eq!(
                abilities.len(),
                1,
                "expected exactly one ability for {t:?}, got {}",
                abilities.len()
            );
            assert_eq!(abilities[0].name(), "test-agent.chat");
        }
    }

    #[test]
    fn chat_manifest_schema_is_object_with_typed_entry_shapes() {
        let entry = entry_named("claude", RuntimeKind::ClaudeCode);
        let manifests = manifests_for("claude", &entry);
        let params = manifests[0].input_schema();
        // Top-level type is an object — every LLM tool-use contract
        // OpenAI / Anthropic / Axon emits assumes this. Verify it
        // here so a future "let me simplify the schema" patch can't
        // silently break tool registration at runtime.
        assert_eq!(
            params.get("type").and_then(Value::as_str),
            Some("object"),
            "schema top-level type must be \"object\""
        );
        let one_of = params
            .get("oneOf")
            .and_then(Value::as_array)
            .expect("oneOf entry-shape guard must be an array");
        assert_eq!(one_of.len(), 2);
        let properties = params
            .get("properties")
            .and_then(Value::as_object)
            .expect("properties must be an object");
        assert!(properties.contains_key("prompt"));
        assert!(properties.contains_key("messages"));
        assert!(properties.contains_key("execution"));
        // `additionalProperties: false` is load-bearing — it turns
        // callers sending unexpected args into a schema error rather
        // than silently dropping them, which makes debugging "why
        // didn't my arg take effect" tractable.
        assert_eq!(
            params.get("additionalProperties"),
            Some(&Value::Bool(false)),
            "schema must reject extra args (additionalProperties: false)"
        );
    }

    #[test]
    fn chat_manifest_parameters_declare_prompt_and_structured_inputs() {
        let entry = entry_named("codex", RuntimeKind::Codex);
        let manifests = manifests_for("codex", &entry);
        let props = manifests[0]
            .input_schema()
            .get("properties")
            .and_then(Value::as_object)
            .expect("properties must be an object");
        assert!(props.contains_key("prompt"));
        assert!(props.contains_key("messages"));
        assert!(props.contains_key("context"));
        assert_eq!(
            props
                .get("prompt")
                .and_then(|p| p.get("type"))
                .and_then(Value::as_str),
            Some("string"),
        );
        assert_eq!(
            props
                .get("context")
                .and_then(|p| p.get("type"))
                .and_then(Value::as_str),
            Some("string"),
        );
    }

    #[test]
    fn different_agent_names_produce_distinct_ability_names() {
        // Sanity: the `<agent>.chat` template must interpolate the
        // right name. A broken template that hardcoded "agent.chat"
        // would silently alias every agent to the same tool name on
        // the wire, which would make discovery and dispatch ambiguous.
        let claude = entry_named("claude", RuntimeKind::ClaudeCode);
        let codex = entry_named("codex", RuntimeKind::Codex);
        let a = abilities_for("claude", &claude);
        let b = abilities_for("codex", &codex);
        assert_ne!(a[0].name(), b[0].name());
        assert_eq!(a[0].name(), "claude.chat");
        assert_eq!(b[0].name(), "codex.chat");
    }

    #[test]
    fn name_shape_cannot_collide_with_network_tool_names() {
        // The network-tool side advertises names without a `.`:
        // "invoke_ability", "list_devices", "deploy_ability", etc.
        // Ability names always contain a `.` because
        // `validate_agent_name` rejects dots inside agent names.
        // Together that makes a name collision structurally impossible.
        let entry = entry_named("claude", RuntimeKind::ClaudeCode);
        let abilities = abilities_for("claude", &entry);
        assert!(abilities[0].name().contains('.'));
        // Mirror a subset of the reserved network-tool names so a
        // future "let me add `invoke.ability` as a network tool" move
        // would trip this test.
        const RESERVED_NETWORK_TOOLS: &[&str] = &[
            "invoke_ability",
            "list_devices",
            "deploy_ability",
            "run_mission",
            "list_all_abilities",
            "send_to_agent",
        ];
        for name in RESERVED_NETWORK_TOOLS {
            assert!(
                !name.contains('.'),
                "network tool names must not contain `.` — {name:?} breaks the shape invariant"
            );
        }
    }

    // ── AgentAbilitySpec::new validation ────────────────────────────────────

    #[test]
    fn new_rejects_empty_name() {
        let err =
            AgentAbilitySpec::new("", "desc", json!({"type": "object"})).expect_err("should err");
        assert!(err.contains("empty"));
        let err = AgentAbilitySpec::new("   ", "desc", json!({"type": "object"}))
            .expect_err("should err");
        assert!(err.contains("empty"));
    }

    #[test]
    fn new_rejects_dotless_name() {
        let err = AgentAbilitySpec::new("chat", "desc", json!({"type": "object"}))
            .expect_err("should err");
        assert!(err.contains("shape"));
    }

    #[test]
    fn new_rejects_non_object_parameters() {
        for bad in [
            json!(null),
            json!(42),
            json!("string"),
            json!(["not", "an", "object"]),
            json!(true),
        ] {
            let err = AgentAbilitySpec::new("agent.chat", "desc", bad).expect_err("should err");
            assert!(
                err.contains("object"),
                "error must mention the shape violation, got {err:?}"
            );
        }
    }

    #[test]
    fn new_accepts_well_formed_inputs() {
        let spec = AgentAbilitySpec::new(
            "claude.chat",
            "send a prompt",
            json!({"type": "object", "properties": {}}),
        )
        .expect("well-formed spec must be accepted");
        assert_eq!(spec.name(), "claude.chat");
        assert_eq!(spec.description(), "send a prompt");
    }

    /// Regression: `AgentAbilitySpec` must be `Clone` so a discovery
    /// consumer can snapshot the spec list without holding a borrow
    /// on the registry. Compile-time check via a noop round-trip.
    #[test]
    fn spec_is_cloneable() {
        let entry = entry_named("claude", RuntimeKind::ClaudeCode);
        let spec = abilities_for("claude", &entry).into_iter().next().unwrap();
        let _copy = spec.clone();
    }

    // ── manifest-driven enumeration ─────────────────────────────────────────
    //
    // Ability enumeration is manifest-only. Every test that expects a
    // network-visible ability creates a real AgentDirectory and points the
    // registry entry at it.

    /// Build a fresh agent root + AgentEntry pointing at it. Returns
    /// the temp root and the entry so the test can poke at both.
    /// The temp root is leaked rather than tracked through a `TempDir`
    /// guard because these tests don't share state across cases and
    /// clutter from a few abandoned temp dirs is acceptable noise.
    fn make_agent_on_disk(name: &str, agent_type: RuntimeKind) -> (std::path::PathBuf, AgentEntry) {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir()
            .join("easynet-abilities-test")
            .join(format!("{name}-{n}-{}", std::process::id()));
        // Clean any leftover from a previous identical run.
        let _ = std::fs::remove_dir_all(&root);
        let spec = AgentSpec::new(name.to_string(), agent_type);
        AgentDirectory::create(&Location::Local { root: root.clone() }, spec)
            .expect("create agent directory");
        let mut entry = AgentEntry::new(agent_type, None);
        entry.root_path = Some(root.clone());
        (root, entry)
    }

    #[test]
    fn manifest_cache_rebuilds_when_a_manifest_changes_on_disk() {
        // The snapshot cache must never serve a stale catalog: editing
        // a manifest changes the stat signature (length and/or mtime)
        // and forces a rebuild on the next read.
        let (root, entry) = make_agent_on_disk("editline", RuntimeKind::ClaudeCode);
        let first = manifests_for("editline", &entry);
        assert_eq!(first.len(), 1, "seeded chat manifest visible");

        let manifest_path = root.join("abilities").join("chat.ability.toml");
        let original = std::fs::read_to_string(&manifest_path).expect("read seeded manifest");
        let edited = original.replace(
            first[0].description(),
            "edited description for cache invalidation",
        );
        assert_ne!(original, edited, "fixture must actually change the file");
        std::fs::write(&manifest_path, edited).expect("rewrite manifest");

        let second = manifests_for("editline", &entry);
        assert_eq!(
            second[0].description(),
            "edited description for cache invalidation",
            "a changed manifest must be re-read, not served from the snapshot"
        );
    }

    #[test]
    fn manifest_cache_is_not_poisoned_by_a_misnamed_request() {
        // A registry row whose name disagrees with the directory's
        // own agent.toml fails closed (empty list) — and must NOT
        // evict the rightful owner's snapshot while doing so.
        let (_root, entry) = make_agent_on_disk("owner", RuntimeKind::ClaudeCode);
        assert_eq!(manifests_for("owner", &entry).len(), 1);
        assert!(
            manifests_for("impostor", &entry).is_empty(),
            "name/directory mismatch fails closed"
        );
        assert_eq!(
            manifests_for("owner", &entry).len(),
            1,
            "the rightful owner's snapshot survives a misnamed request"
        );
    }

    #[test]
    fn manifest_driven_path_returns_chat_from_seeded_manifest() {
        // After `AgentDirectory::create`, `abilities/chat.ability.toml`
        // exists. `abilities_for` must read it — not synthesize.
        let (_root, entry) = make_agent_on_disk("alice", RuntimeKind::ClaudeCode);
        let abilities = abilities_for("alice", &entry);
        assert_eq!(abilities.len(), 1);
        assert_eq!(abilities[0].name(), "alice.chat");
    }

    #[test]
    fn manifest_driven_path_does_not_recreate_deleted_chat_manifest() {
        // If the operator removes chat.ability.toml, discovery must not
        // recreate it through a migration seam. Ability identity is the
        // authored manifest set.
        let (root, entry) = make_agent_on_disk("bob", RuntimeKind::ClaudeCode);
        let chat_path = root
            .join("abilities")
            .join(format!("chat{ABILITY_MANIFEST_SUFFIX}"));
        std::fs::remove_file(&chat_path).expect("remove chat manifest");
        assert!(!chat_path.exists(), "precondition: file removed");
        let abilities = abilities_for("bob", &entry);
        assert!(
            !chat_path.exists(),
            "manifest-only discovery must not rewrite chat.ability.toml"
        );
        assert!(
            abilities.is_empty(),
            "deleted chat manifest must produce no synthetic chat ability: {abilities:?}"
        );
    }

    #[test]
    fn manifest_driven_path_surfaces_extra_abilities_from_manifests() {
        // Drop a second manifest into the abilities directory and
        // verify it shows up alongside chat. This is the forward-
        // compatibility property the whole refactor exists for: an
        // operator who adds `voice.ability.toml` should see it
        // surface in discovery without recompiling the daemon.
        let (root, entry) = make_agent_on_disk("carol", RuntimeKind::ClaudeCode);
        let voice = AbilityManifest::new(
            "voice",
            "Speak a synthesized response.",
            json!({"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}),
        )
        .expect("build voice manifest");
        let voice_path = root
            .join("abilities")
            .join(format!("voice{ABILITY_MANIFEST_SUFFIX}"));
        std::fs::write(&voice_path, voice.to_toml_string().unwrap()).expect("write voice manifest");
        let names: Vec<String> = abilities_for("carol", &entry)
            .into_iter()
            .map(|s| s.name().to_string())
            .collect();
        assert!(
            names.contains(&"carol.chat".to_string()),
            "names = {names:?}"
        );
        assert!(
            names.contains(&"carol.voice".to_string()),
            "names = {names:?}"
        );
    }

    #[test]
    fn manifest_driven_path_uses_manifest_description_not_hardcoded() {
        // The whole point of "chat is a real ability" is that an
        // operator can edit description / schema without touching
        // code. Pin that the manifest's description wins over the
        // hardcoded fallback's interpolated string.
        let (root, entry) = make_agent_on_disk("dave", RuntimeKind::ClaudeCode);
        let edited = AbilityManifest::new(
            "chat",
            "Edited blurb that the operator typed by hand.",
            json!({"type": "object", "properties": {"prompt": {"type": "string"}}, "required": ["prompt"], "additionalProperties": false}),
        )
        .unwrap();
        let chat_path = root
            .join("abilities")
            .join(format!("chat{ABILITY_MANIFEST_SUFFIX}"));
        std::fs::write(&chat_path, edited.to_toml_string().unwrap()).unwrap();
        let abilities = abilities_for("dave", &entry);
        assert_eq!(abilities.len(), 1);
        assert_eq!(
            abilities[0].description(),
            "Edited blurb that the operator typed by hand."
        );
    }

    #[test]
    fn entry_without_root_path_publishes_no_abilities() {
        let entry = AgentEntry::new(RuntimeKind::ClaudeCode, None);
        assert!(entry.root_path.is_none(), "precondition");
        let abilities = abilities_for("ephemeral", &entry);
        assert!(
            abilities.is_empty(),
            "manifest-only discovery must not synthesize chat abilities"
        );
    }
}
