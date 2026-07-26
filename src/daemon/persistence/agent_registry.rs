// EasyNet CLI — Agent Registry
// =============================
//
// File: src/shared/agents.rs
// Description: Persistent registry for AI agent configurations (~/.easynet/agents.json).
//
// Stores agent definitions (Claude Code, Codex, etc.) that can be invoked by
// `easynet agent send` or the multi-agent `easynet discuss` orchestrator.
//
// Separated from config.rs to preserve its three-domain contract
// (runtime state / credentials / device settings).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::core::agent::spec::RuntimeKind;
use crate::daemon::persistence::config;

// ─── Agent Type ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentType {
    ClaudeCode,
    Codex,
    CodexAppServer,
    /// A user-defined external agent runtime. Unlike the LLM-CLI
    /// variants, this one is not tied to a specific binary: the agent's
    /// own `command`/`args` point at any executable whose chat brain reads
    /// the NL prompt on stdin and writes the answer on stdout. This is
    /// the dynamic-extension seam: registering a new harness agent is
    /// configuration, not a new enum variant.
    External,
}

impl std::fmt::Display for AgentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClaudeCode => write!(f, "claude-code"),
            Self::Codex => write!(f, "codex"),
            Self::CodexAppServer => write!(f, "codex-app-server"),
            Self::External => write!(f, "external"),
        }
    }
}

impl std::str::FromStr for AgentType {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "claude-code" | "claude" => Ok(Self::ClaudeCode),
            "codex" => Ok(Self::Codex),
            "codex-app-server" | "codex-appserver" => Ok(Self::CodexAppServer),
            "external" | "custom" => Ok(Self::External),
            _ => anyhow::bail!(
                "unknown agent type: {s} (expected: claude-code, codex, codex-app-server, external)"
            ),
        }
    }
}

impl AgentType {
    pub(crate) fn runtime_kind(self) -> RuntimeKind {
        match self {
            Self::ClaudeCode => RuntimeKind::ClaudeCode,
            Self::Codex => RuntimeKind::Codex,
            Self::CodexAppServer => RuntimeKind::CodexAppServer,
            Self::External => RuntimeKind::External,
        }
    }
}

// ─── Agent Entry ─────────────────────────────────────────────────────────────

/// One row of the on-disk agent registry (`~/.easynet/agents.json`).
///
/// Field visibility policy:
///   - All fields are `pub(crate)` rather than `pub`. Two reasons:
///     (1) serde requires field visibility from *somewhere*, but the
///     downstream surface (other crates) should not see field layout —
///     when we add `tenant: String` later, only this crate has to change.
///     (2) intra-crate readers can still use field access for ergonomics,
///     but the *recommended* read path is the getter methods below, which
///     name-resolve identically to field access at the call site.
///
/// Mutation policy: prefer the typed builders (`with_label`, etc.) over
/// direct field writes — they keep CLI mutation paths consistent with the
/// invariants enforced at registration (`validate_agent_name`). Direct
/// `pub(crate)` field writes are available as the escape hatch for
/// `AgentEntry::new` and for tests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEntry {
    /// On-disk schema version for this registry row. Absent
    /// (serde default `0`) means the row was written by a retired
    /// pre-v2 release. Runtime load rejects those rows; registry
    /// migration is no longer part of the production read path.
    ///
    /// We use a distinct `schema_version` field on `AgentEntry`
    /// (rather than reusing `AgentSpec::schema_version`) because
    /// the two live at different layers: the spec describes an
    /// agent's configuration file on disk; this stamp describes
    /// one row of the registry index. The upgrade cadence of
    /// the two is orthogonal — a registry might need a schema
    /// bump when an index-level field changes (e.g. adding
    /// `root_path`) without any change to the on-disk spec.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub(crate) schema_version: u32,

    /// Path to this agent's on-disk root directory. Having it present is what lets
    /// `daemon::execution::mission::workspace` and `cli::agent list`
    /// resolve where an agent lives without re-computing the
    /// path from `state_dir + name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) root_path: Option<PathBuf>,

    pub(crate) agent_type: AgentType,

    // ── Runtime projection fields ────────────────────────────
    //
    // These fields remain registry projections used by current
    // agent lifecycle paths (for example external runtime command
    // material and UI metadata). They are not a migration source of
    // truth: load-time v1 repair has been retired, and canonical
    // agent root ownership comes only from `root_path`.
    //
    // Bespoke `skip_serializing_if` helpers are named
    // `is_default_*` for grepability: if you're hunting "why
    // does my field still appear in the JSON", start at these.
    #[serde(default = "default_command", skip_serializing_if = "String::is_empty")]
    pub(crate) command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) label: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) env: BTreeMap<String, String>,
    #[serde(
        default = "default_timeout",
        skip_serializing_if = "is_default_timeout"
    )]
    pub(crate) timeout_secs: u64,
    #[serde(
        default = "default_max_output",
        skip_serializing_if = "is_default_max_output"
    )]
    pub(crate) max_output_bytes: usize,
}

fn is_default_timeout(t: &u64) -> bool {
    *t == default_timeout()
}

fn is_default_max_output(n: &usize) -> bool {
    *n == default_max_output()
}

/// Canonical default wall-clock budget for one agent runtime dispatch.
///
/// Fresh v2 rows carry this in `timeout_secs` for shape stability, and
/// `agent.toml` entries that omit `timeout_secs` resolve to this value at
/// dispatch time. Keeping both call sites on one named function prevents the
/// registry row from becoming a second timeout authority.
pub(crate) fn default_timeout_for_new_rows() -> u64 {
    default_timeout()
}

/// Value a fresh v2 row should carry in `max_output_bytes`.
/// Same rationale as `default_timeout_for_new_rows`.
pub(crate) fn default_max_output_for_new_rows() -> usize {
    default_max_output()
}

/// Current registry-row schema version. Bump when adding a
/// field that cannot be defaulted on read.
pub(crate) const CURRENT_REGISTRY_SCHEMA: u32 = 2;

fn is_zero_u32(n: &u32) -> bool {
    *n == 0
}

fn default_command() -> String {
    String::new()
}
fn default_timeout() -> u64 {
    // 1 hour. Per-row deadline for an agent's underlying CLI dispatch
    // (claude / codex). Bumped from 5 min: a real LLM with tool use
    // can legitimately take tens of minutes on a long task (mission
    // think, multi-step agent tool loop, large code-review pass), and
    // a 5 min ceiling was forcing operators to override on nearly
    // every long-running call. Operators who want a tight per-row
    // cap can still set it explicitly via the registry editor.
    3600
}
fn default_max_output() -> usize {
    1_048_576
} // 1 MB

impl AgentEntry {
    // ── Typed builder ─────────────────────────────────────────────────
    //
    // `with_label` is the *only* sanctioned cross-module mutation path
    // for `AgentEntry` after construction. The fields themselves are
    // `pub(crate)` so intra-crate readers can use natural field-access
    // syntax, but writes must go through builders so the day a field
    // gains an invariant (e.g. labels capped at 64 chars), only the
    // builder needs to learn it.
    //
    // Most read-side accessors are intentionally NOT added: every reader in
    // this crate uses field-access syntax against the `pub(crate)`
    // fields, and mirror getters would be redundant noise. `required_root_path`
    // is the exception because root resolution is an ownership boundary: once
    // the registry row has loaded, readers must not rebuild a fallback path
    // from `agents_root()/name`.

    /// Replace the human-readable label. Returns `&mut Self` to allow
    /// the CLI `agent add` path to chain mutations without granting
    /// raw field-write permission to the rest of the crate.
    pub fn with_label(&mut self, label: Option<String>) -> &mut Self {
        self.label = label;
        self
    }

    /// Replace the per-agent model identifier. Same chained-builder
    /// shape as `with_label`; used by the CLI `agent set --model`
    /// path so the registry row reflects the new model alongside
    /// the on-disk `agent.toml` rewrite.
    ///
    /// `None` clears the field — the agent then falls back to
    /// whatever default the underlying CLI (`claude` / `codex`)
    /// picks. Symmetric with `agent add` where `--model` is
    /// optional.
    pub fn with_model(&mut self, model: Option<String>) -> &mut Self {
        self.model = model;
        self
    }

    /// Return the canonical root path for this registry row.
    ///
    /// Missing `root_path` after `load_agents()` is a corrupt registry state,
    /// not permission for callers to infer a default path. Fresh agent creation
    /// may still choose `agents_root()/name` before the row exists; steady-state
    /// readers must go through this method.
    pub(crate) fn required_root_path(&self, name: &str, context: &str) -> anyhow::Result<PathBuf> {
        self.root_path.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "{context}: registered agent {name:?} has no canonical root_path; \
                 recreate the agent or import a canonical registry row before accessing agent state"
            )
        })
    }

    /// Create a new agent entry with sensible defaults for the given type.
    pub fn new(agent_type: AgentType, model: Option<String>) -> Self {
        let (command, args) = match agent_type {
            AgentType::ClaudeCode => (
                "claude".to_string(),
                vec![
                    "-p".to_string(),
                    "--output-format".to_string(),
                    "text".to_string(),
                ],
            ),
            AgentType::Codex => ("codex".to_string(), vec!["exec".to_string()]),
            AgentType::CodexAppServer => ("codex".to_string(), vec!["app-server".to_string()]),
            // External agents have no default binary: the operator
            // supplies `command`/`args` at `agent add` time. Leaving
            // them empty here keeps `new` total without inventing a
            // default that would later look like an executable.
            AgentType::External => (String::new(), Vec::new()),
        };
        Self {
            schema_version: CURRENT_REGISTRY_SCHEMA,
            root_path: None,
            agent_type,
            command,
            args,
            model,
            label: None,
            env: BTreeMap::new(),
            timeout_secs: default_timeout(),
            max_output_bytes: default_max_output(),
        }
    }
}

// ─── Agent Registry ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentRegistry {
    pub agents: BTreeMap<String, AgentEntry>,
}

fn agents_path() -> PathBuf {
    config::state_dir().join("agents.json")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentsFileSignature {
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    ctime_nsec: i64,
    #[cfg(unix)]
    mtime_nsec: i64,
}

#[derive(Debug, Clone)]
struct CachedAgentRegistry {
    signature: Option<AgentsFileSignature>,
    registry: AgentRegistry,
}

fn agent_registry_cache() -> &'static Mutex<HashMap<PathBuf, CachedAgentRegistry>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedAgentRegistry>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn agents_file_signature(path: &Path) -> Option<AgentsFileSignature> {
    let meta = fs::metadata(path).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(AgentsFileSignature {
            len: meta.len(),
            modified: meta.modified().ok(),
            inode: meta.ino(),
            ctime_nsec: meta.ctime_nsec(),
            mtime_nsec: meta.mtime_nsec(),
        })
    }
    #[cfg(not(unix))]
    Some(AgentsFileSignature {
        len: meta.len(),
        modified: meta.modified().ok(),
    })
}

fn cached_agents(path: &Path, signature: &Option<AgentsFileSignature>) -> Option<AgentRegistry> {
    let cache = agent_registry_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache
        .get(path)
        .filter(|entry| &entry.signature == signature)
        .map(|entry| entry.registry.clone())
}

fn store_agents_cache(
    path: &Path,
    signature: Option<AgentsFileSignature>,
    registry: &AgentRegistry,
) {
    let mut cache = agent_registry_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.insert(
        path.to_path_buf(),
        CachedAgentRegistry {
            signature,
            registry: registry.clone(),
        },
    );
}

pub fn load_agents() -> anyhow::Result<AgentRegistry> {
    let path = agents_path();
    let initial_signature = agents_file_signature(&path);
    if let Some(registry) = cached_agents(&path, &initial_signature) {
        return Ok(registry);
    }
    // Read directly and classify the error, rather than `exists()`-then-
    // `read_to_string()`. The two-step form races with `easynet device reset`
    // and `easynet agent remove` running in another terminal: the file
    // disappears between the exists check and the read, producing a
    // misleading "read failed" error when the correct answer is "no
    // registry exists, return default". Matching on `NotFound` gives
    // the same UX without the TOCTOU window.
    let data = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let registry = AgentRegistry::default();
            store_agents_cache(&path, None, &registry);
            return Ok(registry);
        }
        Err(e) => {
            return Err(anyhow::Error::new(e).context(format!("read {}", path.display())));
        }
    };
    if data.trim().is_empty() {
        let registry = AgentRegistry::default();
        store_agents_cache(
            &path,
            agents_file_signature(&path).or(initial_signature),
            &registry,
        );
        return Ok(registry);
    }
    let registry: AgentRegistry =
        serde_json::from_str(&data).with_context(|| format!("parse {}", path.display()))?;

    validate_loaded_registry(&registry)?;

    store_agents_cache(
        &path,
        agents_file_signature(&path).or(initial_signature),
        &registry,
    );
    Ok(registry)
}

fn validate_loaded_registry(registry: &AgentRegistry) -> anyhow::Result<()> {
    for (name, entry) in &registry.agents {
        validate_agent_name(name)
            .with_context(|| format!("validate registered agent key `{name}`"))?;
        if entry.schema_version != CURRENT_REGISTRY_SCHEMA {
            anyhow::bail!(
                "agent registry row `{name}` has unsupported schema_version {}; expected {}; \
                 pre-v2 registry migration is retired, so recreate the agent or import a \
                 canonical row with root_path",
                entry.schema_version,
                CURRENT_REGISTRY_SCHEMA
            );
        }
        if entry.root_path.is_none() {
            anyhow::bail!(
                "agent registry row `{name}` is missing canonical root_path; \
                 runtime no longer infers agent roots from agents_root/name"
            );
        }
    }
    Ok(())
}

/// Validate an agent name before it lands in the registry.
///
/// Agent names flow from this registry into:
/// 1. the A2A discovery label `a2a.agents_json` (see `shared/a2a_labels.rs`),
/// 2. the agent root path `~/.easynet/agents/<name>/`,
/// 3. the codex/claude `--agent <name>` argument,
/// 4. EAL member-call syntax (`<name>.chat(...)`).
///
/// All four surfaces assume a constrained character set. We reject anything
/// that would break path joins, shell argv, or the `a2a.*` reserved
/// prefix at the *registration* boundary so the bad input never reaches a
/// downstream consumer.
pub fn validate_agent_name(name: &str) -> anyhow::Result<()> {
    let agent_id = crate::core::agent::id::AgentId::parse(name)
        .map_err(|error| anyhow::anyhow!("agent registry key {name:?} is invalid: {error}"))?;
    let canonical = agent_id.to_string();
    if canonical != name {
        anyhow::bail!("agent registry key {name:?} is not canonical; expected {canonical:?}");
    }
    for segment in [agent_id.tenant.as_str(), agent_id.name.as_str()] {
        // Reserved prefixes — the `a2a.*` namespace is owned by the A2A
        // label schema (`shared/a2a_labels.rs`), and `easynet*` is
        // reserved for the built-in MCP server identity. Both rules block
        // the trivial collision case where a user names an agent after a
        // system identifier.
        if segment.starts_with("a2a") || segment.starts_with("easynet") {
            anyhow::bail!(
                "agent registry key {name:?} uses a reserved prefix ('a2a*' or 'easynet*')"
            );
        }
    }
    Ok(())
}

pub fn save_agents(registry: &AgentRegistry) -> anyhow::Result<()> {
    // Validate every key once at the persistence boundary, so every code
    // path that builds a registry (CLI add, programmatic insert, manual
    // edit followed by re-save) gets the same guarantees.
    for name in registry.agents.keys() {
        validate_agent_name(name)?;
    }
    let path = agents_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(registry)? + "\n";
    // Reuse the race-safe primitive in `config`. The previous open-coded
    // `path.with_extension("tmp"); write; rename` raced with itself: two
    // concurrent `easynet agent add` invocations both staged to the same
    // `.tmp` path, and writer B's contents could overwrite the file
    // between writer A's stage and rename. See iteration-1 audit notes.
    //
    // The registry can contain `env` entries with API tokens, so we ask
    // for owner-only permissions applied at stage time — a post-rename
    // chmod would leave a TOCTOU window where the file is briefly
    // world-readable at its final path.
    config::atomic_write_with_permissions(
        &path,
        json.as_bytes(),
        config::WritePermissions::OwnerReadWrite,
    )?;
    store_agents_cache(&path, agents_file_signature(&path), registry);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_agent_type_round_trips_and_has_no_default_command() {
        // Dynamic-extension contract: `external` parses, displays, and
        // serializes as "external", and carries no built-in binary.
        let t: AgentType = "external".parse().unwrap();
        assert_eq!(t, AgentType::External);
        assert_eq!(AgentType::External.to_string(), "external");
        assert_eq!("custom".parse::<AgentType>().unwrap(), AgentType::External);
        let entry = AgentEntry::new(AgentType::External, None);
        assert!(entry.command.is_empty());
        assert!(entry.args.is_empty());
    }

    #[test]
    fn validate_agent_name_accepts_well_formed_names() {
        for name in [
            "default/claude",
            "default/codex",
            "research/claude-2",
            "default/my_agent",
            "default/a",
            "default/agent42",
        ] {
            assert!(
                validate_agent_name(name).is_ok(),
                "expected '{name}' to be accepted"
            );
        }
    }

    #[test]
    fn validate_agent_name_rejects_empty_and_oversize() {
        assert!(validate_agent_name("").is_err());
        let too_long = "a".repeat(64);
        assert!(validate_agent_name(&too_long).is_err());
    }

    #[test]
    fn validate_agent_name_rejects_path_and_shell_metachars() {
        // Each of these would either break a path join, escape an EAL
        // member-call, or get re-interpreted by a shell expansion — block
        // them at the registration boundary.
        for bad in [
            "claude",
            "default/claude/foo",
            "../etc",
            "default/../etc",
            "agent.name", // dot would collide with EAL `<agent>.chat` syntax
            "Claude",     // uppercase rejected for canonicalization
            "agent name",
            "agent;rm",
            "agent$VAR",
            "agent\n",
            "agent🤖", // non-ASCII rejected
        ] {
            assert!(
                validate_agent_name(bad).is_err(),
                "expected '{bad}' to be rejected"
            );
        }
    }

    #[test]
    fn validate_agent_name_rejects_reserved_prefixes() {
        // `a2a.*` is the discovery-label namespace; `easynet*` is the
        // built-in MCP server identity. Both must not be shadowable by a
        // user-registered agent.
        for reserved in [
            "default/a2a",
            "default/a2a-clone",
            "default/easynet",
            "default/easynet-fake",
            "a2a/team",
            "easynet/team",
        ] {
            assert!(
                validate_agent_name(reserved).is_err(),
                "expected reserved name '{reserved}' to be rejected"
            );
        }
    }

    // ── Registry load boundary ────────────────────────────────────────────
    //
    // Each test isolates to a temp HOME via `HomeGuard` so validation never
    // touches the developer's real registry.

    use crate::cli::commands::test_support::HomeGuard;

    fn seed_registry(contents: &str) -> PathBuf {
        let dir = config::state_dir();
        fs::create_dir_all(&dir).unwrap();
        let path = agents_path();
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn load_rejects_retired_pre_v2_row_without_migration() {
        let _g = HomeGuard::new();
        seed_registry(
            r#"{
                "agents": {
                    "default/alice": {
                        "agent_type": "claude-code",
                        "command": "claude",
                        "args": ["-p"],
                        "model": "claude-opus-4-7",
                        "timeout_secs": 600
                    }
                }
            }"#,
        );
        let error = load_agents().expect_err("pre-v2 registry rows must fail closed");
        assert!(
            error.to_string().contains("unsupported schema_version 0"),
            "{error:#}"
        );
        assert!(
            !agents_path().with_extension("json.v1.bak").exists(),
            "load must not create a migration backup"
        );
    }

    #[test]
    fn load_rejects_current_row_without_canonical_root_path() {
        let _g = HomeGuard::new();
        seed_registry(
            r#"{
                "agents": {
                    "default/alice": {
                        "schema_version": 2,
                        "agent_type": "claude-code",
                        "command": "claude"
                    }
                }
            }"#,
        );
        let error = load_agents().expect_err("missing root_path must fail closed");
        assert!(
            error.to_string().contains("missing canonical root_path"),
            "{error:#}"
        );
    }

    #[test]
    fn load_rejects_malformed_registered_agent_key() {
        let _g = HomeGuard::new();
        seed_registry(
            r#"{
                "agents": {
                    "../alice": {
                        "schema_version": 2,
                        "root_path": "/tmp/easynet-test-alice",
                        "agent_type": "claude-code"
                    }
                }
            }"#,
        );
        let error = load_agents().expect_err("invalid registry key must fail closed");
        assert!(
            error.to_string().contains("validate registered agent key"),
            "{error:#}"
        );
    }

    #[test]
    fn v2_row_passes_through_load_unchanged() {
        // A registry written by this binary on a fresh install must pass
        // through without migration side effects.
        let _g = HomeGuard::new();
        seed_registry(
            r#"{
                "agents": {
                    "default/alice": {
                        "schema_version": 2,
                        "root_path": "/tmp/easynet-test-alice",
                        "agent_type": "claude-code",
                        "command": "claude"
                    }
                }
            }"#,
        );
        let reg = load_agents().unwrap();
        assert_eq!(reg.agents["default/alice"].schema_version, 2);
        let bak = agents_path().with_extension("json.v1.bak");
        assert!(!bak.exists(), "v2-on-disk registry must not trigger backup");
    }

    #[test]
    fn load_agents_cache_observes_external_registry_rewrite() {
        let _g = HomeGuard::new();
        seed_registry(
            r#"{
                "agents": {
                    "default/alice": {
                        "schema_version": 2,
                        "root_path": "/tmp/easynet-test-alice",
                        "agent_type": "claude-code"
                    }
                }
            }"#,
        );
        let first = load_agents().unwrap();
        assert!(first.agents.contains_key("default/alice"));

        seed_registry(
            r#"{
                "agents": {
                    "default/bravo": {
                        "schema_version": 2,
                        "root_path": "/tmp/easynet-test-bravo",
                        "agent_type": "codex"
                    }
                }
            }"#,
        );
        let second = load_agents().unwrap();
        assert!(!second.agents.contains_key("default/alice"));
        assert!(second.agents.contains_key("default/bravo"));
    }

    #[test]
    fn registry_key_and_agent_spec_name_are_separate_boundaries() {
        use crate::core::agent::spec::{AgentSpec, RuntimeKind};

        assert!(validate_agent_name("default/claude").is_ok());
        assert!(validate_agent_name("claude").is_err());

        let mut spec = AgentSpec::new("placeholder", RuntimeKind::ClaudeCode);
        spec.name = "claude".to_string();
        assert!(spec.validate().is_ok());

        spec.name = "default/claude".to_string();
        assert!(spec.validate().is_err());
    }
}
