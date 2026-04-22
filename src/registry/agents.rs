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

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::core::agent_spec::{AgentSpec, RuntimeKind};
use crate::persistence::config;

// ─── Agent Type ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentType {
    ClaudeCode,
    Codex,
    CodexAppServer,
}

impl std::fmt::Display for AgentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClaudeCode => write!(f, "claude-code"),
            Self::Codex => write!(f, "codex"),
            Self::CodexAppServer => write!(f, "codex-app-server"),
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
            _ => anyhow::bail!(
                "unknown agent type: {s} (expected: claude-code, codex, codex-app-server)"
            ),
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
    /// (serde default `0`) means the row was written by a
    /// pre-v2 release — load-path migration in
    /// `migrate_registry_to_v2` upgrades it to v2 on the next
    /// read. Written rows always carry `schema_version = 2`.
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

    /// Path to this agent's on-disk root directory. v2-only
    /// field: absent on v1 rows (load-path fills it from the
    /// legacy workspace path). Having it present is what lets
    /// `runtime::workspace` and `facade::cli::agent list`
    /// resolve where an agent lives without re-computing the
    /// path from `state_dir + name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) root_path: Option<PathBuf>,

    pub(crate) agent_type: AgentType,

    // ── Fat v1 fields ────────────────────────────────────────
    //
    // These fields carry the pre-v2 shape. The read path keeps
    // them fully functional (migration pulls their values into
    // agent.toml + .env before clearing). The write path skips
    // them when they hold their default/empty value — which is
    // exactly what a fresh v2 `agent new` writes today. That
    // lets the JSON file on disk slim down to the v2 shape
    // without a second pass, while any still-fat row read from
    // a hand-edited file round-trips its fat fields back.
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
    #[serde(default = "default_timeout", skip_serializing_if = "is_default_timeout")]
    pub(crate) timeout_secs: u64,
    #[serde(default = "default_max_output", skip_serializing_if = "is_default_max_output")]
    pub(crate) max_output_bytes: usize,
}

fn is_default_timeout(t: &u64) -> bool {
    *t == default_timeout()
}

fn is_default_max_output(n: &usize) -> bool {
    *n == default_max_output()
}

/// Value a fresh v2 row should carry in `timeout_secs`. Exposed
/// so `facade::cli::agent::run_add` can set it explicitly rather
/// than rely on `AgentEntry::new`'s current behaviour — see the
/// rationale block in `run_add` for why that symmetry matters.
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
    300
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
    // Read-side accessors are intentionally NOT added: every reader in
    // this crate uses field-access syntax against the `pub(crate)`
    // fields, and adding mirror getters would be either dead code (the
    // compiler flags it) or redundant noise. If a downstream crate ever
    // needs to *read* a field, add a getter at that point — not now.

    /// Replace the human-readable label. Returns `&mut Self` to allow
    /// the CLI `agent add` path to chain mutations without granting
    /// raw field-write permission to the rest of the crate.
    pub fn with_label(&mut self, label: Option<String>) -> &mut Self {
        self.label = label;
        self
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

pub fn load_agents() -> anyhow::Result<AgentRegistry> {
    let path = agents_path();
    // Read directly and classify the error, rather than `exists()`-then-
    // `read_to_string()`. The two-step form races with `easynet reset`
    // and `easynet agent remove` running in another terminal: the file
    // disappears between the exists check and the read, producing a
    // misleading "read failed" error when the correct answer is "no
    // registry exists, return default". Matching on `NotFound` gives
    // the same UX without the TOCTOU window.
    let data = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AgentRegistry::default());
        }
        Err(e) => {
            return Err(anyhow::Error::new(e).context(format!("read {}", path.display())));
        }
    };
    if data.trim().is_empty() {
        return Ok(AgentRegistry::default());
    }
    let mut registry: AgentRegistry =
        serde_json::from_str(&data).with_context(|| format!("parse {}", path.display()))?;

    // Load-time v1 → v2 migration.
    //
    // Pre-v2 rows have `schema_version == 0` (the absent-field
    // default). If any such row is present we run the migration
    // here, on read, so the next caller sees a fully v2 registry.
    // Rationale for doing this on read (rather than in a dedicated
    // `easynet migrate` command):
    //
    //   * The fleet of call sites that read the registry is wide
    //     (every CLI command that touches an agent). Demanding a
    //     separate migration run would leave a failure mode where
    //     post-upgrade CLI invocations behave inconsistently until
    //     the operator remembers to run the command.
    //   * The migration is cheap and idempotent — v2 rows are
    //     passed through unchanged. Running it on every load costs
    //     nothing in steady state.
    //
    // If migration itself fails we return the error without
    // writing anything back: the caller sees a loud failure
    // rather than a half-migrated registry. The `.v1.bak` backup
    // created by `ensure_v1_backup` before any mutation means a
    // rollback-and-retry path exists even in that case.
    if registry.agents.values().any(|e| e.schema_version == 0) {
        migrate_registry_to_v2(&mut registry)?;
        // Write the migrated shape back so subsequent loads never
        // re-run the migration. `save_agents` is atomic; a crash
        // between the migration and the save leaves the on-disk
        // file in its v1 state, which the next run will re-migrate
        // — safe because migration is idempotent on its own inputs.
        save_agents(&registry)?;
    }

    Ok(registry)
}

/// v1 → v2 registry migration.
///
/// For each v1 row (`schema_version == 0`) we:
/// 1. Resolve the agent's on-disk root: the legacy path
///    `~/.easynet/workspaces/<name>/` if it exists, otherwise
///    `config::agents_root().join(<name>)` which — per PR-0b —
///    falls back to the legacy tree when only that exists.
/// 2. If the root does not already hold an `agent.toml`, build a
///    minimal `AgentSpec` from the v1 fields (runtime, model,
///    timeout_secs) and materialise the directory via
///    `AgentDirectory::create`. `AgentDirectory::create`'s own
///    partial-skeleton guard protects us from a second-pass
///    retry adopting a stale layout from a failed earlier
///    migration.
/// 3. If the v1 row carries `env` entries, write them to
///    `<agent-root>/.env` (chmod 600 on unix — handled by
///    `AgentDirectory::create` when it creates the file fresh).
///    Emit a stdout notice listing the key names (not the
///    values) so an operator grepping their scrollback can see
///    which agent just had credentials migrated.
/// 4. Stamp the registry row: `schema_version = 2`,
///    `root_path = Some(<root>)`. Other fields stay — future
///    PRs will clean them up once the workspace-projection and
///    dispatch call sites no longer read them.
///
/// Pre-flight: before touching anything we ensure a v1 backup
/// exists at `~/.easynet/agents.json.v1.bak`. `ensure_v1_backup`
/// is idempotent — the backup is written once and preserved
/// across multiple migration attempts so a rollback remains
/// possible.
fn migrate_registry_to_v2(registry: &mut AgentRegistry) -> anyhow::Result<()> {
    ensure_v1_backup().context("back up pre-v2 registry")?;

    // Collect the names needing migration up front. We don't
    // iterate-and-mutate because the migration mutates the row we
    // just looked up, and the borrow checker would force an
    // awkward dance. Names are cheap to clone.
    let legacy_names: Vec<String> = registry
        .agents
        .iter()
        .filter(|(_, e)| e.schema_version == 0)
        .map(|(name, _)| name.clone())
        .collect();

    for name in legacy_names {
        // The row we just identified must still be present —
        // `legacy_names` was computed from a snapshot of this
        // registry and nothing has mutated it yet.
        let entry = registry
            .agents
            .get_mut(&name)
            .expect("entry present per legacy_names snapshot");

        migrate_one_entry(&name, entry)
            .with_context(|| format!("migrate agent `{name}` from v1 to v2"))?;
    }

    Ok(())
}

/// Migrate one v1 row into v2 form. Split from the driver loop
/// so each row's failure has a clean `with_context(..)` anchor
/// in the top-level error.
fn migrate_one_entry(name: &str, entry: &mut AgentEntry) -> anyhow::Result<()> {
    use crate::runtime::directory::{AgentDirectory, Location};

    // Resolve where this agent's root should live. The
    // `agents_root()` helper already folds over the new-or-legacy
    // fallback introduced in PR-0b.
    let root = config::agents_root().join(name);

    // Build a spec that captures the v1 state without loss.
    let runtime = match entry.agent_type {
        AgentType::ClaudeCode => RuntimeKind::ClaudeCode,
        AgentType::Codex => RuntimeKind::Codex,
        AgentType::CodexAppServer => RuntimeKind::CodexAppServer,
    };
    let mut spec = AgentSpec::new(name, runtime);
    spec.model = entry.model.clone();
    // v1 timeout defaults (300) are the same as the runtime
    // default, so we only persist `timeout_secs` when the v1 row
    // customized it. This keeps migrated agent.toml files minimal.
    if entry.timeout_secs != default_timeout() {
        spec.timeout_secs = Some(entry.timeout_secs);
    }
    if let Some(label) = &entry.label {
        spec.description = Some(label.clone());
    }
    // `env` is handled after directory creation so the .env file
    // the directory creates fresh receives it atomically.

    // Materialise the directory. If it already exists with an
    // agent.toml we're in a "re-migration of an already-migrated
    // directory" case, which happens when the registry file was
    // hand-edited back to v1 shape but the spec on disk is
    // already v2; we skip the create in that case.
    let existing_toml = root.join("agent.toml");
    if !existing_toml.exists() {
        AgentDirectory::create(&Location::Local { root: root.clone() }, spec.clone())
            .with_context(|| format!("create agent directory at {}", root.display()))?;
    }

    // Write any v1 env entries to `<agent-root>/.env`. The file
    // was just created fresh with 0o600 by `AgentDirectory::create`
    // (unix) so the permission-hardening step is implicit. We
    // append key=value lines — a deliberately simple format, not
    // shell-quoted; operators who need complex values can edit
    // the file by hand after migration.
    if !entry.env.is_empty() {
        let env_path = root.join(".env");
        let mut contents = String::new();
        for (k, v) in &entry.env {
            // Only emit keys that round-trip cleanly as unquoted
            // .env lines. Anything with a newline or `=` in the
            // key would corrupt the format; we warn and skip
            // rather than silently produce an unparseable file.
            if k.contains('\n') || k.contains('=') || v.contains('\n') {
                eprintln!(
                    "[easynet migrate] agent `{name}`: skipping env var `{k}` \
                     (value contains newline or `=`; migrate manually)"
                );
                continue;
            }
            contents.push_str(k);
            contents.push('=');
            contents.push_str(v);
            contents.push('\n');
        }
        if !contents.is_empty() {
            config::atomic_write_with_permissions(
                &env_path,
                contents.as_bytes(),
                config::WritePermissions::OwnerReadWrite,
            )
            .with_context(|| format!("write {}", env_path.display()))?;

            // Stderr, not stdout: this is a diagnostic side
            // effect of `load_agents`, not a result the caller
            // pipes into further processing. Keeping every
            // migration notice on stderr makes `tail -f` of
            // CLI output coherent (compare `ensure_v1_backup`
            // which also emits on stderr) and keeps `stdout`
            // clean for commands whose value is a parsable
            // payload. We name the keys (not the values) so an
            // operator can see which agent just had credentials
            // migrated without printing the credentials
            // themselves.
            let keys: Vec<&str> = entry.env.keys().map(|s| s.as_str()).collect();
            eprintln!(
                "[easynet migrate] agent `{name}`: moved {} env var(s) ({}) to {}",
                entry.env.len(),
                keys.join(", "),
                env_path.display()
            );
        }
    }

    // Stamp the registry row as v2. We clear `env` on the
    // registry row because the source of truth is now the .env
    // file; leaving it in the JSON would create a two-writer
    // problem where a future edit to .env silently disagrees
    // with the registry.
    entry.schema_version = CURRENT_REGISTRY_SCHEMA;
    entry.root_path = Some(root);
    entry.env.clear();

    Ok(())
}

/// Write `~/.easynet/agents.json.v1.bak` once, if absent. Used
/// by `migrate_registry_to_v2` as its only form of durable
/// rollback: if the migration corrupts the registry (or an
/// operator simply wants to re-inspect the v1 shape) the file
/// is a byte-for-byte copy of the pre-migration state.
///
/// Idempotent: a `.v1.bak` already present is preserved as-is,
/// even if the current `agents.json` has been overwritten. The
/// backup corresponds to "first observation of v1 content by
/// this CLI release" and we deliberately do not re-take it on
/// later migrations — a second migration (should one ever
/// happen) would have its own backup target (`.v2.bak`, etc.).
fn ensure_v1_backup() -> anyhow::Result<()> {
    let src = agents_path();
    let bak = src.with_extension("json.v1.bak");
    if bak.exists() {
        return Ok(());
    }
    if !src.exists() {
        // No registry yet to back up (fresh install); no-op.
        return Ok(());
    }
    let data = fs::read(&src).with_context(|| format!("read {}", src.display()))?;
    config::atomic_write_with_permissions(
        &bak,
        &data,
        config::WritePermissions::OwnerReadWrite,
    )
    .with_context(|| format!("write {}", bak.display()))?;
    eprintln!(
        "[easynet migrate] backed up pre-v2 registry to {}",
        bak.display()
    );
    Ok(())
}

/// Validate an agent name before it lands in the registry.
///
/// Agent names flow from this registry into:
/// 1. the A2A discovery label `a2a.agents_json` (see `shared/a2a_labels.rs`),
/// 2. the workspace path `~/.easynet/workspaces/<name>/`,
/// 3. the codex/claude `--agent <name>` argument,
/// 4. EAL member-call syntax (`<name>.chat(...)`).
///
/// All four surfaces assume a constrained character set. We reject anything
/// that would break path joins, shell argv, or the `a2a.*` reserved
/// prefix at the *registration* boundary so the bad input never reaches a
/// downstream consumer.
pub fn validate_agent_name(name: &str) -> anyhow::Result<()> {
    if name.is_empty() {
        anyhow::bail!("agent name must not be empty");
    }
    if name.len() > 63 {
        anyhow::bail!(
            "agent name '{name}' is too long ({} chars; max 63)",
            name.len()
        );
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        anyhow::bail!(
            "agent name '{name}' must contain only lowercase ASCII letters, digits, '_', or '-'"
        );
    }
    // Reserved prefixes — the `a2a.*` namespace is owned by the A2A label
    // schema (`shared/a2a_labels.rs`), and `easynet*` is reserved for the
    // built-in MCP server identity. Both rules block the trivial collision
    // case where a user names an agent after a system identifier.
    if name.starts_with("a2a") || name.starts_with("easynet") {
        anyhow::bail!("agent name '{name}' uses a reserved prefix ('a2a*' or 'easynet*')");
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_agent_name_accepts_well_formed_names() {
        for name in ["claude", "codex", "claude-2", "my_agent", "a", "agent42"] {
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
            "claude/foo",
            "../etc",
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
        for reserved in ["a2a", "a2a-clone", "easynet", "easynet-fake"] {
            assert!(
                validate_agent_name(reserved).is_err(),
                "expected reserved name '{reserved}' to be rejected"
            );
        }
    }

    // ── v1 → v2 migration ────────────────────────────────────────────────
    //
    // Each test isolates to a temp HOME via `HomeGuard` so
    // migration never touches the developer's real registry.
    // The cases cover: migration happens on load, is
    // idempotent, writes a .v1.bak backup exactly once, moves
    // v1 env to `<agent-root>/.env`, and preserves non-env
    // fields on the registry row.

    use crate::facade::cli::test_support::HomeGuard;

    /// Compose a v1-shaped agents.json on disk. Each caller
    /// tunes the shape; returns the absolute path.
    fn seed_v1_registry(contents: &str) -> PathBuf {
        let dir = config::state_dir();
        fs::create_dir_all(&dir).unwrap();
        let path = agents_path();
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn load_migrates_v1_row_to_v2_and_stamps_schema_version() {
        // A plain v1 row (no schema_version, has fat fields) must
        // come back from `load_agents` carrying schema_version=2
        // and a resolved root_path.
        let _g = HomeGuard::new();
        seed_v1_registry(
            r#"{
                "agents": {
                    "alice": {
                        "agent_type": "claude-code",
                        "command": "claude",
                        "args": ["-p"],
                        "model": "claude-opus-4-7",
                        "timeout_secs": 600
                    }
                }
            }"#,
        );
        let reg = load_agents().expect("migration must succeed");
        let alice = reg.agents.get("alice").expect("alice present");
        assert_eq!(alice.schema_version, CURRENT_REGISTRY_SCHEMA);
        assert!(alice.root_path.is_some(), "root_path must be populated");
        // Agent directory must exist with a real agent.toml.
        let root = alice.root_path.as_ref().unwrap();
        assert!(root.join("agent.toml").exists());
    }

    #[test]
    fn load_migration_is_idempotent() {
        // Second load must not re-migrate: the file is already
        // v2, so the `any(schema_version == 0)` branch is skipped.
        // We detect the second-pass skip by observing that the
        // .v1.bak file is not overwritten with whatever the
        // current registry looks like.
        let _g = HomeGuard::new();
        seed_v1_registry(
            r#"{
                "agents": {
                    "alice": {
                        "agent_type": "claude-code",
                        "command": "claude"
                    }
                }
            }"#,
        );

        // First load triggers migration + backup.
        let _ = load_agents().unwrap();
        let bak_path = agents_path().with_extension("json.v1.bak");
        let bak_before = fs::read_to_string(&bak_path).unwrap();
        assert!(bak_before.contains("\"agents\""));

        // Mutate the registry on disk (simulate a fresh
        // `agent add` writing a second v2 row). Then load again.
        let current = fs::read_to_string(agents_path()).unwrap();
        assert!(current.contains("\"schema_version\": 2"));
        let _ = load_agents().unwrap();

        // Backup must be byte-identical to its first write. If
        // the second load had re-entered migration, the backup
        // would now be a copy of the post-migration v2 file.
        let bak_after = fs::read_to_string(&bak_path).unwrap();
        assert_eq!(bak_before, bak_after, "backup must not change on re-load");
    }

    #[test]
    fn v1_env_moves_to_dotenv_file_and_clears_from_registry() {
        // The critical credential-migration path: env vars on
        // the v1 row must end up in `<agent-root>/.env` (chmod
        // 600 on unix) and be removed from the registry so no
        // downstream reader sees stale credentials after the
        // file has been edited by hand.
        let _g = HomeGuard::new();
        seed_v1_registry(
            r#"{
                "agents": {
                    "alice": {
                        "agent_type": "claude-code",
                        "command": "claude",
                        "env": {
                            "ANTHROPIC_API_KEY": "sk-real",
                            "CUSTOM_FLAG": "1"
                        }
                    }
                }
            }"#,
        );
        let reg = load_agents().unwrap();
        let alice = &reg.agents["alice"];
        // Registry row's env must be cleared.
        assert!(alice.env.is_empty(), "registry env must be cleared on migrate");

        // .env file must contain both vars.
        let root = alice.root_path.as_ref().unwrap();
        let env_body = fs::read_to_string(root.join(".env")).unwrap();
        assert!(env_body.contains("ANTHROPIC_API_KEY=sk-real"));
        assert!(env_body.contains("CUSTOM_FLAG=1"));

        // Unix: .env must be 0o600 after migration wrote it. The
        // path went through `atomic_write_with_permissions` which
        // matches the AgentDirectory::create permission.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(root.join(".env"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "migrated .env must be 0o600, got {mode:o}");
        }
    }

    #[test]
    fn v1_backup_is_written_on_first_migration_only() {
        // `.v1.bak` captures the pre-migration state. If we run
        // migration twice (by seeding a v1 registry, loading,
        // then deliberately reseeding a v1 registry to force a
        // second migration), the backup from the first run must
        // be preserved, not overwritten.
        let _g = HomeGuard::new();
        seed_v1_registry(
            r#"{
                "agents": {
                    "first": {"agent_type": "claude-code", "command": "claude"}
                }
            }"#,
        );
        let _ = load_agents().unwrap();
        let bak_path = agents_path().with_extension("json.v1.bak");
        let first_bak = fs::read_to_string(&bak_path).unwrap();
        assert!(first_bak.contains("\"first\""));

        // Force a second migration: hand-edit the registry back
        // to v1 shape with a different agent. The migration will
        // run again on the next load, but `ensure_v1_backup`
        // must see the existing `.v1.bak` and not overwrite it.
        seed_v1_registry(
            r#"{
                "agents": {
                    "second": {"agent_type": "codex", "command": "codex"}
                }
            }"#,
        );
        let _ = load_agents().unwrap();
        let second_bak = fs::read_to_string(&bak_path).unwrap();
        assert_eq!(
            first_bak, second_bak,
            "backup must record only the first observed v1 state"
        );
    }

    #[test]
    fn v1_row_without_env_still_migrates_cleanly() {
        // Edge: a v1 row with no env entries must not write an
        // empty `.env` full of garbage or otherwise misbehave.
        // The AgentDirectory::create path already creates an
        // empty `.env` with 0o600; migration must leave that
        // untouched.
        let _g = HomeGuard::new();
        seed_v1_registry(
            r#"{
                "agents": {
                    "alice": {"agent_type": "claude-code", "command": "claude"}
                }
            }"#,
        );
        let reg = load_agents().unwrap();
        let alice = &reg.agents["alice"];
        let root = alice.root_path.as_ref().unwrap();
        let env_body = fs::read_to_string(root.join(".env")).unwrap();
        // The AgentDirectory::create-generated .env is empty;
        // migration with no env entries must keep it that way.
        assert!(
            env_body.is_empty(),
            ".env must be empty when v1 row had no env; got {env_body:?}"
        );
    }

    #[test]
    fn v2_row_passes_through_load_unchanged() {
        // A registry that is already v2 (e.g. written by this
        // binary on a fresh install) must not trigger the
        // migration branch or write a backup. We verify by
        // asserting `.v1.bak` does not exist after load.
        let _g = HomeGuard::new();
        seed_v1_registry(
            r#"{
                "agents": {
                    "alice": {
                        "schema_version": 2,
                        "root_path": "/tmp/easynet-test-alice",
                        "agent_type": "claude-code",
                        "command": "claude"
                    }
                }
            }"#,
        );
        let reg = load_agents().unwrap();
        assert_eq!(reg.agents["alice"].schema_version, 2);
        let bak = agents_path().with_extension("json.v1.bak");
        assert!(
            !bak.exists(),
            "v2-on-disk registry must not trigger backup"
        );
    }

    #[test]
    fn v1_timeout_default_is_not_persisted_in_spec() {
        // A v1 row with `timeout_secs = 300` (the v1 default) must
        // produce an agent.toml without an explicit
        // `timeout_secs` field. That keeps migrated files minimal
        // and honours the "spec records explicit user choice"
        // principle: a default that happens to match is not a
        // choice, it's the absence of one.
        let _g = HomeGuard::new();
        seed_v1_registry(
            r#"{
                "agents": {
                    "alice": {
                        "agent_type": "claude-code",
                        "command": "claude",
                        "timeout_secs": 300
                    }
                }
            }"#,
        );
        let reg = load_agents().unwrap();
        let root = reg.agents["alice"].root_path.as_ref().unwrap();
        let toml = fs::read_to_string(root.join("agent.toml")).unwrap();
        assert!(
            !toml.contains("timeout_secs"),
            "default timeout must not be persisted; got:\n{toml}"
        );
    }

    #[test]
    fn v1_custom_timeout_is_persisted_in_spec() {
        // The converse: a v1 row with a non-default timeout
        // must carry the value forward into agent.toml, because
        // that *is* an explicit user choice.
        let _g = HomeGuard::new();
        seed_v1_registry(
            r#"{
                "agents": {
                    "alice": {
                        "agent_type": "claude-code",
                        "command": "claude",
                        "timeout_secs": 900
                    }
                }
            }"#,
        );
        let reg = load_agents().unwrap();
        let root = reg.agents["alice"].root_path.as_ref().unwrap();
        let toml = fs::read_to_string(root.join("agent.toml")).unwrap();
        assert!(
            toml.contains("timeout_secs = 900"),
            "custom timeout must be persisted; got:\n{toml}"
        );
    }

    #[test]
    fn v1_label_migrates_to_spec_description() {
        // v1 had a thin `label: Option<String>` field we used
        // as the human-readable agent description. v2 spec
        // carries this as `description`. Migration must bridge
        // so the EasyNet Frontend card doesn't suddenly lose
        // descriptions for every legacy-registered agent.
        let _g = HomeGuard::new();
        seed_v1_registry(
            r#"{
                "agents": {
                    "alice": {
                        "agent_type": "claude-code",
                        "command": "claude",
                        "label": "senior code reviewer"
                    }
                }
            }"#,
        );
        let reg = load_agents().unwrap();
        let root = reg.agents["alice"].root_path.as_ref().unwrap();
        let toml = fs::read_to_string(root.join("agent.toml")).unwrap();
        assert!(
            toml.contains("description = \"senior code reviewer\""),
            "label must migrate to description; got:\n{toml}"
        );
    }

    #[test]
    fn migration_env_with_embedded_newline_is_skipped_with_warning() {
        // Credentials with embedded newlines would corrupt the
        // .env line-per-var format. Migration must skip them
        // (rather than silently producing unparseable .env) and
        // leave the rest intact. Operators see the skip on
        // stderr.
        let _g = HomeGuard::new();
        seed_v1_registry(
            r#"{
                "agents": {
                    "alice": {
                        "agent_type": "claude-code",
                        "command": "claude",
                        "env": {
                            "GOOD": "plain-value",
                            "BAD": "has\nnewline"
                        }
                    }
                }
            }"#,
        );
        let reg = load_agents().unwrap();
        let root = reg.agents["alice"].root_path.as_ref().unwrap();
        let env_body = fs::read_to_string(root.join(".env")).unwrap();
        assert!(env_body.contains("GOOD=plain-value"));
        assert!(
            !env_body.contains("BAD"),
            "newline-bearing value must be skipped; got {env_body:?}"
        );
    }

    /// Cross-module parity: every rule in
    /// `registry::agents::validate_agent_name` must also be enforced
    /// by `core::agent_spec::AgentSpec::validate`. If the two drift
    /// (e.g. someone tightens this side but not the core side), a
    /// user can write an `agent.toml` whose name parses locally but
    /// gets rejected at registry insertion — a confusing half-state.
    /// We pin the parity by feeding the registry's reject set into
    /// the spec's validate and asserting every case also errors.
    ///
    /// Note we do NOT assert the error *messages* match — they are
    /// allowed to differ (the spec names the file, the registry
    /// names the name). Only the accept/reject decision matters.
    #[test]
    fn agent_spec_and_registry_agent_name_rules_agree() {
        use crate::core::agent_spec::{AgentSpec, RuntimeKind};

        // Names that MUST be rejected by both sides.
        let rejects = [
            // empty
            "",
            // path / shell metas
            "claude/foo",
            "../etc",
            "agent.name",
            "agent name",
            "agent;rm",
            "agent$VAR",
            "agent\n",
            // non-ASCII / uppercase
            "Claude",
            "agent🤖",
            // reserved prefixes
            "a2a",
            "a2a-clone",
            "easynet",
            "easynet-fake",
        ];
        for bad in rejects {
            let registry_err = validate_agent_name(bad).is_err();
            let spec_err = {
                let mut s = AgentSpec::new("placeholder", RuntimeKind::ClaudeCode);
                s.name = bad.to_string();
                s.validate().is_err()
            };
            assert!(
                registry_err && spec_err,
                "parity broken on {bad:?}: registry_err={registry_err}, spec_err={spec_err}"
            );
        }

        // Names that MUST be accepted by both sides.
        let accepts = ["claude", "codex", "claude-2", "my_agent", "a", "agent42"];
        for good in accepts {
            let registry_ok = validate_agent_name(good).is_ok();
            let spec_ok = {
                let mut s = AgentSpec::new("placeholder", RuntimeKind::ClaudeCode);
                s.name = good.to_string();
                s.validate().is_ok()
            };
            assert!(
                registry_ok && spec_ok,
                "parity broken on {good:?}: registry_ok={registry_ok}, spec_ok={spec_ok}"
            );
        }
    }
}
