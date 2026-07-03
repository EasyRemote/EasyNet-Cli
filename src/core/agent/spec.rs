// EasyNet CLI — Agent Specification (agent.toml)
// ================================================
//
// File: src/core/agent_spec.rs
// Description: On-disk schema for `<agent-root>/agent.toml` — the
//              source of truth for a single agent's configuration.
//
// Where this fits in the stack
// ----------------------------
// An `AgentSpec` is the typed representation of one agent's
// `agent.toml`. It names the agent, points at a runtime (the CLI
// binary that drives it: claude-code, codex, …), and carries the
// user-tuneable knobs that shape how that runtime behaves for this
// specific agent. It is deliberately **data-only**: every field is
// either a String, an Option<String>, or a small BTreeMap. No
// `Path`s, no file handles, no network.
//
// The consumer of this type is `AgentDirectory` (future), which
// projects an `AgentSpec` onto the on-disk layout a runtime expects
// (`.mcp.json` for Claude Code, `.codex/config.toml` for Codex,
// `CLAUDE.md` / `AGENTS.md` context files, etc.). This file does
// not know about that projection — that would put layering in
// reverse.
//
// Why this lives in `core/`
// -------------------------
// `core/` is the zero-dependency ontology layer. `AgentSpec` is a
// pure data type that every other subsystem reads (registry to
// locate, runtime to drive, publish to advertise). Putting it any
// lower in the stack would create a cycle; putting it higher (e.g.
// under `runtime/`) would leak transport/process concerns down to
// its call sites.
//
// What is NOT in this file
// ------------------------
// * The `AgentDirectory` struct that walks the filesystem, creates
//   a fresh layout, or lists existing agents — lives in
//   `daemon::execution::mission::directory` (future PR).
// * The `AgentRegistry` entry that resolves `name -> path`. That
//   still lives in `registry::agents` for now; a future PR
//   replaces today's `AgentEntry::{command, args, model, env, …}`
//   with a thin `{ name, root_path, runtime }` row whose fields
//   all redirect into the `AgentSpec` on disk.
// * Any subprocess command resolution. Runtimes carry their own
//   binary discovery (`runtime::drivers::claude_code::doctor`,
//   etc.); the spec only names the runtime, never a bare command.
//
// Layering rule
// -------------
// `core::agent_spec` must not import any other `crate::` module
// and must not pull in external crates beyond `serde` + `toml`.
// Violations will be caught at review — the whole point of core/
// is that lower layers can import it without pulling in the
// world.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Which runtime binary drives this agent. A runtime is the CLI
/// process the operator has installed locally (Claude Code, Codex
/// exec, Codex app-server, …). It is orthogonal to the agent's
/// identity — many agents on one machine can share a runtime.
///
/// Wire / disk encoding uses kebab-case (`claude-code`, not
/// `claude_code` or `ClaudeCode`) to match the existing
/// `crate::daemon::persistence::agent_registry::AgentType` display form. That parity
/// is load-bearing: the registry and the on-disk spec must agree
/// on the spelling, or a user who reads one file and writes the
/// other will get a phantom "unknown runtime" at load time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeKind {
    ClaudeCode,
    Codex,
    CodexAppServer,
    /// User-defined external-process runtime. The agent's own
    /// `command`/`args` point at any executable that answers an NL
    /// prompt over stdin/stdout. The dynamic-extension seam: new harness
    /// agents are configuration, not new enum variants.
    External,
}

impl RuntimeKind {
    /// Stable wire form used by both the on-disk TOML and the
    /// `a2a.agents_json[*].type` discovery label. Mirrors
    /// `AgentType::to_string()` so switching a caller from the
    /// registry's enum to the spec's enum cannot change the
    /// observable string.
    pub fn as_wire_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Codex => "codex",
            Self::CodexAppServer => "codex-app-server",
            Self::External => "external",
        }
    }
}

impl std::fmt::Display for RuntimeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_wire_str())
    }
}

/// The schema version this binary understands natively. An
/// `agent.toml` carrying `schema_version = "1"` (or omitting the
/// field entirely) is the "current implicit schema" — everything
/// written by this release. Future releases that add fields which
/// older consumers cannot interpret will bump this to `"2"` and
/// teach `validate` to accept the new value. The const is the
/// single source of truth so a future bump flips exactly one
/// place.
pub const CURRENT_SCHEMA_VERSION: &str = "1";

/// Schema versions this binary accepts on read. Kept narrow on
/// purpose: accepting unknown versions would silently swallow a
/// future incompatible layout. When a real `"2"` lands (PR that
/// introduces new required fields), the implementer adds `"2"`
/// here and writes the load-path migration to go with it.
const SUPPORTED_SCHEMA_VERSIONS: &[&str] = &["1"];

/// One agent's persistent configuration, as serialized to
/// `<agent-root>/agent.toml`.
///
/// Field policy
/// ------------
/// * `name` and `runtime` are **required**. Everything else is
///   optional because a fresh spec produced by `agent new`
///   must be usable with nothing but those two set.
/// * Every optional value in the TOML has a sensible default at
///   the consumer layer (runtime defaults, registry defaults). We
///   do NOT bake those defaults into the spec itself — the spec
///   is a pure record of what the user wrote down, and conflating
///   "user set it to X" with "user accepted the default" loses
///   information the diagnostic commands (`agent doctor`,
///   `agent show`) need.
/// * `env` intentionally carries a `BTreeMap` (not `HashMap`) so
///   the serialized TOML has a stable key order. Two writes of
///   the same semantic content must produce byte-identical files
///   — otherwise unrelated patches show up as diffs in a
///   project-local agent root under git.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSpec {
    /// Self-describing schema version. Optional on read: an absent
    /// field means "this file was written before the version stamp
    /// was introduced" and is treated as `"1"` (the current
    /// implicit schema). A future PR that adds required fields
    /// will bump `CURRENT_SCHEMA_VERSION` to `"2"`, add it to
    /// `SUPPORTED_SCHEMA_VERSIONS`, and teach `validate` / the
    /// load-path how to upgrade a `"1"` on the fly.
    ///
    /// Why `Option<String>` rather than `String` with a default:
    /// distinguishing "the user omitted this field" from "the user
    /// wrote exactly the current version" matters for diffs — two
    /// byte-identical agent.tomls must continue to survive
    /// round-trip, and serde's `#[serde(default)]` would normalize
    /// every read into an emitted field that wasn't there before.
    /// We rely on `skip_serializing_if = "Option::is_none"` so a
    /// spec that came in without the field goes back out without
    /// it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<String>,

    /// Agent's local name. Must match the key the registry uses
    /// for this agent. Not a full `AgentId` — tenant resolution
    /// is the registry's job, not the spec's.
    pub name: String,

    /// Which runtime binary drives this agent.
    pub runtime: RuntimeKind,

    /// Preferred model identifier passed to the runtime. Freeform
    /// because the runtime owns its own model catalog; the spec
    /// is only a channel for the operator's choice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Runtime-specific interaction mode (e.g. `"default"`,
    /// `"acceptEdits"`, `"plan"`, `"bypassPermissions"` for
    /// Claude Code; `"auto"`, `"full-access"` for Codex). The
    /// spec does not validate the value — each runtime driver
    /// interprets its own legal set at load time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,

    /// System-prompt-style preamble the runtime prepends before
    /// every user prompt. Newlines preserved verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,

    /// Allow-list of runtime tool names this agent may invoke.
    /// `None` means "runtime default" (no restriction beyond
    /// whatever the runtime enforces itself). An empty Vec means
    /// "no tools at all" — a deliberate narrowing, distinct from
    /// the absent case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<Vec<String>>,

    /// Human-readable description; shown in `agent list` and the
    /// EasyNet Frontend's agent card. Freeform.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Who owns this agent (email or registry handle). Used by
    /// the EasyNet Frontend to group agents in the UI; not
    /// enforced by the CLI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,

    /// Hard wall-clock budget per invocation, in seconds. `None`
    /// falls back to the runtime's default (see
    /// `registry::agents::default_timeout`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,

    /// Environment variables the runtime should see. Kept on the
    /// spec (rather than in a sibling `.env` file) because it is
    /// the simplest migration path for v1 `AgentEntry::env`
    /// records: the follow-up PR can move these out to a chmod-600
    /// `.env` file without changing the wire shape of the spec.
    /// BTreeMap for stable serialized ordering.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

impl AgentSpec {
    /// Build a minimal spec with just the two required fields.
    /// Every other field defaults to "absent"; the TOML writer
    /// will omit them on disk so a fresh `agent new` produces
    /// the smallest possible file.
    ///
    /// Note `schema_version` defaults to `None` here, not
    /// `Some(CURRENT_SCHEMA_VERSION)`. Rationale: a spec
    /// constructed programmatically and serialized back should
    /// produce the smallest possible file (none of the version
    /// stamps, just name + runtime). Readers that need to know
    /// the effective version call `effective_schema_version()`
    /// below.
    pub fn new(name: impl Into<String>, runtime: RuntimeKind) -> Self {
        Self {
            schema_version: None,
            name: name.into(),
            runtime,
            model: None,
            mode: None,
            system_prompt: None,
            allowed_tools: None,
            description: None,
            owner: None,
            timeout_secs: None,
            env: BTreeMap::new(),
        }
    }

    /// Resolve the effective schema version: either the value
    /// the operator wrote, or the current implicit default.
    /// Exposed as a helper so readers that branch on version
    /// (future migration code) don't have to open-code the
    /// `None → "1"` fallback everywhere.
    pub fn effective_schema_version(&self) -> &str {
        self.schema_version
            .as_deref()
            .unwrap_or(CURRENT_SCHEMA_VERSION)
    }

    /// Parse `agent.toml` contents. Errors carry enough context
    /// that a caller can surface a user-facing "line N, field
    /// foo" message — we rely on the `toml` crate's built-in
    /// error spans for that, plus an `anyhow` wrapper so the
    /// caller's own context (file path) can be attached.
    pub fn from_toml_str(src: &str) -> anyhow::Result<Self> {
        let spec: Self = toml::from_str(src)?;
        spec.validate()?;
        Ok(spec)
    }

    /// Serialize back to a TOML document. Stable ordering
    /// guaranteed by serde + our BTreeMap choice for `env`.
    pub fn to_toml_string(&self) -> anyhow::Result<String> {
        let s = toml::to_string_pretty(self)?;
        Ok(s)
    }

    /// Enforce invariants the serde derive cannot express on its
    /// own. Kept separate from deserialization so a caller that
    /// constructs a spec programmatically (tests, `agent new`)
    /// can validate without round-tripping through TOML.
    ///
    /// Rules (must mirror `registry::agents::validate_agent_name`)
    /// --------------------------------------------------------
    /// * Non-empty, length ≤ 63 chars.
    /// * Character set: `[a-z0-9_-]` only. This is deliberately
    ///   narrower than the TOML spec allows — an agent name
    ///   flows into filesystem paths (`<agent-root>`), shell
    ///   argv (`--agent <name>`), EAL member-call syntax
    ///   (`<name>.chat`), and the `a2a.agents_json` label
    ///   discovery field. Every one of those surfaces would
    ///   misbehave on uppercase, whitespace, non-ASCII, or
    ///   path/shell metacharacters, so we block them at the
    ///   ingestion boundary.
    /// * Reserved prefixes `a2a*` and `easynet*` are rejected —
    ///   those namespaces are owned by the discovery label
    ///   schema and the built-in MCP server identity
    ///   respectively.
    ///
    /// Why we duplicate `registry::agents::validate_agent_name`
    /// rather than call it: `core::` cannot import `registry::`
    /// without creating a cycle. The duplication is pinned by
    /// the `agent_spec_and_registry_agent_name_rules_agree`
    /// test in the registry module, so a future tightening on
    /// one side that forgets the other trips the test loudly.
    pub fn validate(&self) -> anyhow::Result<()> {
        let n = &self.name;
        if n.is_empty() {
            anyhow::bail!("agent.toml: `name` is empty");
        }
        if n.len() > 63 {
            anyhow::bail!("agent.toml: `name` is too long ({} chars; max 63)", n.len());
        }
        if !n
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        {
            // Name common offenders in the error so operators know
            // what to fix without reading the source. The path/dot
            // hints matter because those are the two everyday
            // mistakes (copy-paste from `tenant/name`, typing the
            // ability-form `<agent>.chat` by accident).
            anyhow::bail!(
                "agent.toml: `name` = {:?} must contain only lowercase ASCII letters, \
                 digits, `_`, or `-` (no `/`, no `.`, no whitespace, no uppercase, no non-ASCII)",
                n
            );
        }
        if n.starts_with("a2a") || n.starts_with("easynet") {
            anyhow::bail!(
                "agent.toml: `name` = {:?} uses a reserved prefix (`a2a*` or `easynet*`)",
                n
            );
        }

        // Schema version gate. An absent field is the pre-stamp
        // shape (accepted as `"1"`); any string present must be in
        // `SUPPORTED_SCHEMA_VERSIONS`. Silently accepting unknown
        // versions would let a `"3"` file from a newer release
        // deserialize into this binary's `"1"` layout, losing any
        // required fields the future release added. Refusing here
        // instead lets the operator see "this CLI is too old, use
        // a newer release" rather than a silent semantic drift.
        if let Some(v) = self.schema_version.as_deref() {
            if !SUPPORTED_SCHEMA_VERSIONS.contains(&v) {
                anyhow::bail!(
                    "agent.toml: schema_version = {:?} is not supported by this binary \
                     (supported: {:?}). Upgrade the CLI or remove the field to default to \
                     the current implicit schema.",
                    v,
                    SUPPORTED_SCHEMA_VERSIONS
                );
            }
        }

        // `timeout_secs = 0` is a footgun, not an expressive choice.
        //
        // A reader of the field naturally reads "0 = no limit" or "0 =
        // fail fast". Neither is what `Duration::from_secs(0)` actually
        // means downstream: when the runtime hands it to `Command::wait_
        // timeout` (or the equivalent in `process_runner`), zero seconds
        // is a *zero* wait window — the child is SIGKILLed immediately
        // after spawn, before stdin can even be written. Operators who
        // set this value will get "every call is instantly killed",
        // which is neither of the two intents they meant to express.
        //
        // We block the ambiguity at the ingestion boundary (same
        // nursery principle we apply to name-character rules above),
        // so no runtime ever has to guess what the user meant. A
        // caller who truly wants "fail fast" can set e.g. `1` and get
        // the real boundary the runtime supports.
        if self.timeout_secs == Some(0) {
            anyhow::bail!(
                "agent.toml: `timeout_secs = 0` is rejected (would SIGKILL the child \
                 immediately; use a positive value or omit the field to take the \
                 runtime default)"
            );
        }

        Ok(())
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! Each test pins one invariant the module's doc contract
    //! promises. Failure messages name the invariant so a broken
    //! report tells the maintainer which promise just slipped.

    use super::*;

    // ── happy path ──────────────────────────────────────────────────────────

    #[test]
    fn minimal_spec_serializes_with_only_required_fields() {
        // A spec built via `AgentSpec::new` carries only `name`
        // and `runtime`. Every other field is absent; the TOML
        // output must reflect that — no `model = ""`, no
        // `description = ""`, no empty `[env]` table.
        let s = AgentSpec::new("alice", RuntimeKind::ClaudeCode);
        let toml = s.to_toml_string().unwrap();
        assert!(toml.contains("name = \"alice\""));
        assert!(toml.contains("runtime = \"claude-code\""));
        // None of the optional fields may appear.
        for unexpected in [
            "schema_version",
            "model",
            "mode",
            "system_prompt",
            "allowed_tools",
            "description",
            "owner",
            "timeout_secs",
            "[env]",
        ] {
            assert!(
                !toml.contains(unexpected),
                "minimal spec leaked field {unexpected:?} into output:\n{toml}"
            );
        }
    }

    #[test]
    fn round_trip_preserves_every_field() {
        // An operator-populated spec with every knob set must
        // survive write → read → compare unchanged. If this
        // fails, one of the serde attributes has drifted away
        // from the field's semantic meaning.
        let mut env = BTreeMap::new();
        env.insert("API_KEY".into(), "xyz".into());
        env.insert("DEBUG".into(), "1".into());

        let original = AgentSpec {
            schema_version: Some(CURRENT_SCHEMA_VERSION.into()),
            name: "senior-reviewer".into(),
            runtime: RuntimeKind::Codex,
            model: Some("gpt-5".into()),
            mode: Some("plan".into()),
            system_prompt: Some("be terse\nuse bullets".into()),
            allowed_tools: Some(vec!["read".into(), "grep".into()]),
            description: Some("Strict code reviewer.".into()),
            owner: Some("silan.hu@u.nus.edu".into()),
            timeout_secs: Some(120),
            env,
        };

        let written = original.to_toml_string().unwrap();
        let read_back = AgentSpec::from_toml_str(&written).unwrap();
        assert_eq!(read_back, original);
    }

    #[test]
    fn serialized_env_keys_are_lexicographically_stable() {
        // Two semantically identical specs must produce the same
        // bytes — this is the property that keeps project-local
        // agent roots from flapping in `git diff` when nothing
        // functional changed.
        let mut a = AgentSpec::new("x", RuntimeKind::ClaudeCode);
        let mut b = AgentSpec::new("x", RuntimeKind::ClaudeCode);
        // Insert in different orders; BTreeMap's serialization
        // should render them identically.
        a.env.insert("Z".into(), "1".into());
        a.env.insert("A".into(), "2".into());
        b.env.insert("A".into(), "2".into());
        b.env.insert("Z".into(), "1".into());
        assert_eq!(a.to_toml_string().unwrap(), b.to_toml_string().unwrap());
    }

    #[test]
    fn runtime_wire_strings_match_registry_agent_type() {
        // Load-bearing parity: the spec's on-disk runtime string
        // must equal the registry's AgentType::to_string(). If a
        // refactor ever changes one without the other, a user's
        // agent.toml written today would fail to load tomorrow.
        // We encode the contract as a literal comparison so a
        // future kebab-vs-snake flip trips the test loudly.
        assert_eq!(RuntimeKind::ClaudeCode.as_wire_str(), "claude-code");
        assert_eq!(RuntimeKind::Codex.as_wire_str(), "codex");
        assert_eq!(
            RuntimeKind::CodexAppServer.as_wire_str(),
            "codex-app-server"
        );
        assert_eq!(RuntimeKind::External.as_wire_str(), "external");
    }

    // ── failure path ────────────────────────────────────────────────────────

    #[test]
    fn reject_missing_name_field() {
        let src = r#"
            runtime = "claude-code"
        "#;
        let err = AgentSpec::from_toml_str(src).expect_err("missing `name` must error");
        let msg = format!("{err}");
        assert!(
            msg.contains("name") || msg.contains("missing"),
            "error must name the missing field; got {msg}"
        );
    }

    #[test]
    fn reject_missing_runtime_field() {
        let src = r#"
            name = "alice"
        "#;
        let err = AgentSpec::from_toml_str(src).expect_err("missing `runtime` must error");
        let msg = format!("{err}");
        assert!(
            msg.contains("runtime") || msg.contains("missing"),
            "error must name the missing field; got {msg}"
        );
    }

    #[test]
    fn reject_unknown_runtime_string() {
        // A typo like `runtime = "claude_code"` (snake instead of
        // kebab) must not be silently accepted — mis-routing to
        // the wrong driver silently is worse than a loud error.
        let src = r#"
            name = "alice"
            runtime = "claude_code"
        "#;
        let err = AgentSpec::from_toml_str(src).expect_err("typo must not be accepted");
        assert!(
            format!("{err}").to_lowercase().contains("runtime")
                || format!("{err}").to_lowercase().contains("variant")
        );
    }

    #[test]
    fn validate_rejects_empty_name() {
        // Caught by `validate`, not by serde — empty string is
        // valid TOML, so we need the invariant layer.
        let mut s = AgentSpec::new("alice", RuntimeKind::ClaudeCode);
        s.name.clear();
        let err = s.validate().expect_err("empty name must error");
        assert!(format!("{err}").contains("empty"));
    }

    #[test]
    fn validate_rejects_slash_in_name() {
        let s = AgentSpec::new("team/alice", RuntimeKind::ClaudeCode);
        let err = s.validate().expect_err("slash must error");
        assert!(format!("{err}").contains("/"));
    }

    #[test]
    fn validate_rejects_dot_in_name() {
        // A dot would collide with the `<agent>.<ability>` shape
        // the publish layer relies on.
        let s = AgentSpec::new("claude.extra", RuntimeKind::ClaudeCode);
        let err = s.validate().expect_err("dot must error");
        assert!(format!("{err}").contains("."));
    }

    #[test]
    fn validate_rejects_leading_whitespace_in_name() {
        // Invisible-leading-tab is a real class of bug in
        // user-editable TOML files. Catch it at the ingestion
        // boundary rather than at the first `agent send` that
        // tries to resolve the name.
        let s = AgentSpec::new("\talice", RuntimeKind::ClaudeCode);
        let err = s.validate().expect_err("leading whitespace must error");
        assert!(format!("{err}").contains("whitespace") || format!("{err}").contains("control"));
    }

    #[test]
    fn from_toml_str_applies_validate() {
        // Serde-level parsing alone does not enforce invariants.
        // `from_toml_str` must compose with `validate()` so
        // callers get one-stop loading.
        let src = r#"
            name = "team/alice"
            runtime = "claude-code"
        "#;
        let err =
            AgentSpec::from_toml_str(src).expect_err("validator must run after deserialization");
        assert!(format!("{err}").contains("/"));
    }

    // ── edge cases ──────────────────────────────────────────────────────────

    #[test]
    fn empty_allowed_tools_vec_is_distinct_from_absent() {
        // An explicit empty list ("deny everything") must
        // round-trip as itself, not collapse to `None` ("runtime
        // default"). Confusing the two is a silent policy
        // regression.
        let mut s = AgentSpec::new("alice", RuntimeKind::ClaudeCode);
        s.allowed_tools = Some(Vec::new());
        let toml = s.to_toml_string().unwrap();
        let back = AgentSpec::from_toml_str(&toml).unwrap();
        assert_eq!(back.allowed_tools, Some(Vec::new()));
        assert_ne!(back.allowed_tools, None);
    }

    #[test]
    fn unicode_description_survives_round_trip() {
        // Operators write prose in their native language in the
        // description field; a serializer that fumbles non-ASCII
        // here would corrupt every EasyNet Frontend card.
        let s = AgentSpec {
            schema_version: None,
            name: "alice".into(),
            runtime: RuntimeKind::ClaudeCode,
            description: Some("高级代码审查员 — 对 Rust / TypeScript 项目尤其严格。🔍".into()),
            model: None,
            mode: None,
            system_prompt: None,
            allowed_tools: None,
            owner: None,
            timeout_secs: None,
            env: BTreeMap::new(),
        };
        let toml = s.to_toml_string().unwrap();
        let back = AgentSpec::from_toml_str(&toml).unwrap();
        assert_eq!(back.description, s.description);
    }

    #[test]
    fn system_prompt_preserves_multiline_content() {
        // A system prompt usually spans multiple lines and may
        // include characters that the TOML writer must quote
        // (backslashes, quotes). Verifying the literal content
        // survives round-trip pins that the `toml` crate's
        // string encoding works for the real shape users write.
        let prompt = "Line 1\nLine 2 with \"quotes\"\nLine 3 \\ backslash";
        let mut s = AgentSpec::new("alice", RuntimeKind::ClaudeCode);
        s.system_prompt = Some(prompt.into());
        let toml = s.to_toml_string().unwrap();
        let back = AgentSpec::from_toml_str(&toml).unwrap();
        assert_eq!(back.system_prompt.as_deref(), Some(prompt));
    }

    #[test]
    fn unknown_top_level_keys_are_ignored_for_forward_compat() {
        // TOML allows extra keys; the serde derive drops them by
        // default. We DO want that permissive behavior: a
        // newer version of the binary that adds a field must not
        // break when an operator later downgrades and reads
        // their own file. Pin the contract so a future
        // `#[serde(deny_unknown_fields)]` flip is a deliberate
        // decision, not an accident.
        let src = r#"
            name = "alice"
            runtime = "claude-code"
            future_field = "from-next-release"
        "#;
        let s = AgentSpec::from_toml_str(src)
            .expect("unknown keys must be tolerated for forward compat");
        assert_eq!(s.name, "alice");
        assert_eq!(s.runtime, RuntimeKind::ClaudeCode);
    }

    #[test]
    fn timeout_zero_is_rejected_as_footgun() {
        // Previously this test accepted `Some(0)` under a "don't
        // clamp, let runtime decide" rationale. That framing was
        // false-neutral: there is no sensible runtime
        // interpretation of a zero-second wait window — every
        // concrete consumer (`Command::wait_timeout`,
        // `process_runner`) treats it as "kill immediately after
        // spawn". Operators reading the knob read "fast fail" or
        // "no limit"; neither matches the real behaviour. We
        // block the ambiguity at ingestion so no runtime has to
        // guess.
        let mut s = AgentSpec::new("alice", RuntimeKind::ClaudeCode);
        s.timeout_secs = Some(0);
        let err = s.validate().expect_err("timeout_secs=0 must be rejected");
        let msg = format!("{err}");
        assert!(
            msg.contains("timeout_secs"),
            "error must name the offending field; got {msg}"
        );
        assert!(
            msg.contains("0"),
            "error must mention the rejected value; got {msg}"
        );
    }

    #[test]
    fn timeout_none_and_positive_values_are_both_accepted() {
        // None = "take the runtime default"; any positive value
        // is an explicit choice. These two are the only accepted
        // shapes for timeout_secs and must continue to round-trip
        // cleanly — the rejection above must not accidentally
        // also catch Some(1) or None.
        for tv in [None, Some(1), Some(60), Some(86_400)] {
            let mut s = AgentSpec::new("alice", RuntimeKind::ClaudeCode);
            s.timeout_secs = tv;
            s.validate()
                .unwrap_or_else(|e| panic!("timeout_secs={tv:?} must be accepted, got error: {e}"));
            let toml = s.to_toml_string().unwrap();
            let back = AgentSpec::from_toml_str(&toml).unwrap();
            assert_eq!(back.timeout_secs, tv);
        }
    }

    // ── schema_version ──────────────────────────────────────────────────────

    #[test]
    fn schema_version_absent_round_trips_without_emitting() {
        // A spec built without `schema_version` must serialize
        // without the field, parse back to `None`, and still
        // report the current version as the effective one. This
        // is the "pre-stamp compatibility" contract — an
        // agent.toml written by an earlier release before the
        // stamp existed must not grow the field after a read /
        // write cycle, or every first-open of a legacy file
        // would be a git-visible modification.
        let s = AgentSpec::new("alice", RuntimeKind::ClaudeCode);
        assert!(s.schema_version.is_none());

        let toml = s.to_toml_string().unwrap();
        assert!(
            !toml.contains("schema_version"),
            "None must not emit `schema_version`, got:\n{toml}"
        );

        let back = AgentSpec::from_toml_str(&toml).unwrap();
        assert!(back.schema_version.is_none());
        assert_eq!(back.effective_schema_version(), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn schema_version_explicit_one_is_accepted_and_preserved() {
        // An operator who writes `schema_version = "1"`
        // explicitly must see it round-trip verbatim — the
        // serializer does NOT canonicalize "1" to absent even
        // though the two are semantically equivalent. Reason:
        // "user wrote it" and "user omitted it" are different
        // diffs in a project-local agent root; collapsing them
        // would produce git noise on first-write.
        let mut s = AgentSpec::new("alice", RuntimeKind::ClaudeCode);
        s.schema_version = Some(CURRENT_SCHEMA_VERSION.into());
        s.validate().unwrap();

        let toml = s.to_toml_string().unwrap();
        assert!(toml.contains("schema_version"));

        let back = AgentSpec::from_toml_str(&toml).unwrap();
        assert_eq!(back.schema_version.as_deref(), Some(CURRENT_SCHEMA_VERSION));
        assert_eq!(back.effective_schema_version(), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn schema_version_unknown_value_is_rejected() {
        // A spec carrying a future version the current binary
        // does not understand MUST fail on load. Silently
        // accepting it would deserialize into this binary's v1
        // layout, losing any required fields the newer release
        // added and producing a subtle data-loss bug on first
        // save. The error message names both the offending value
        // and the supported list so an operator can fix the file
        // or upgrade the CLI without reading source.
        let src = r#"
            schema_version = "2"
            name = "alice"
            runtime = "claude-code"
        "#;
        let err =
            AgentSpec::from_toml_str(src).expect_err("unknown schema_version must be rejected");
        let msg = format!("{err}");
        assert!(
            msg.contains("schema_version"),
            "error must name the field; got {msg}"
        );
        assert!(
            msg.contains("\"2\""),
            "error must quote the offending value; got {msg}"
        );
        assert!(
            msg.contains("supported"),
            "error must list supported versions; got {msg}"
        );
    }

    #[test]
    fn effective_schema_version_never_returns_empty() {
        // Defensive: even a caller who sets `schema_version =
        // Some(String::new())` (semantically "no version") must
        // be caught at `validate`. We don't want
        // `effective_schema_version()` to ever hand back an
        // empty string to a downstream consumer.
        let mut s = AgentSpec::new("alice", RuntimeKind::ClaudeCode);
        s.schema_version = Some(String::new());
        // Empty string is not in SUPPORTED_SCHEMA_VERSIONS, so
        // validate must refuse it. This keeps the reachable
        // space for `effective_schema_version()` confined to
        // non-empty strings from the supported list plus the
        // static `CURRENT_SCHEMA_VERSION` fallback.
        assert!(s.validate().is_err());
    }
}
