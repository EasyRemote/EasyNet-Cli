// EasyNet CLI — Ability Manifest (abilities/*.ability.toml)
// ==========================================================
//
// File: src/core/ability_spec.rs
// Description: On-disk schema for one file under
//              `<agent-root>/abilities/<name>.ability.toml`. One
//              manifest per file; the stem of the file name is
//              authoritative for the ability's verb portion.
//
// Where this fits in the stack
// ----------------------------
// An `AbilityManifest` is the typed representation of a single
// `abilities/<verb>.ability.toml`. One agent has many manifests,
// kept as independent files rather than one combined manifest so
// that adding, editing, and removing a single ability is a
// single-file operation (friendly to `git diff`, friendly to
// `mv`/`rm` refactors, friendly to a future `agent publish --only
// <verb>` workflow).
//
// Who reads this
// --------------
// * `runtime::directory` enumerates the files on disk.
// * `publish` (dry-run in PR-4; live in PR-5b) converts each manifest
//   into the `<agent>.<verb>` ToolSpec it would register on an
//   `AbilityToolAdapter`.
// * `a2a_labels` emits the discovery JSON under
//   `a2a.agents_json[*].skills` from the same manifests.
//
// Why this lives in `core/`
// -------------------------
// `core/` is the zero-dependency ontology layer. `AbilityManifest`
// is a pure data type; every other subsystem reads it. Layering
// any higher (e.g. under `runtime/`) would make `registry/` and
// `publish/` import-cycle against `runtime/`. Everything typed
// ends up funneling through this struct, so it belongs at the
// bottom.
//
// What is NOT in this file
// ------------------------
// * Filesystem enumeration — `runtime::directory` walks
//   `<agent-root>/abilities/` and parses each file.
// * Invocation plumbing — `publish/` turns a manifest into a
//   tool-registration call; `runtime::dispatch` does the actual
//   subprocess wrangling when an invocation lands.
// * Any agent-name awareness — the manifest does NOT know which
//   agent it belongs to. The agent name is contributed by the
//   enclosing directory: `<agent>` + `<verb>` → `<agent>.<verb>`
//   is assembled one layer up. That keeps a manifest file
//   portable across `cp -R` of an agent root.
//
// Layering rule
// -------------
// `core::ability_spec` must not import any other `crate::` module
// and must not pull in external crates beyond `serde` + `toml` +
// `serde_json` (for the embedded JSON Schema fields).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Current on-disk schema version. Matches `AgentSpec`'s versioning
/// policy — a single digit, bumped only on a breaking change to the
/// shape. Reader refuses unknown versions; writer stamps the
/// `CURRENT_SCHEMA_VERSION` on every file it generates.
pub const CURRENT_SCHEMA_VERSION: &str = "1";

/// Two kinds of ability the CLI publishes. Introduced by PR-SYS to
/// disambiguate agent abilities (per-agent `<agent>.chat`-style)
/// from device-level system abilities (`system.<feature>`).
///
/// Why an enum and not a free-form string: a name beginning with
/// `system.` is a wire-level promise that the publishing node
/// itself owns the handler — no agent subprocess gets reached. The
/// enum lets a reader look at one field and know which dispatch
/// path applies, instead of grepping the prefix.
///
/// `Agent` is the existing case (every `<name>.chat` pre-PR-SYS
/// shipped under this kind, even though the kind didn't exist
/// then). `System` is the new case enabled by PR-SYS — the daemon
/// publishes the handler, no agent involved. Future kinds (e.g.
/// `Skill` for installable skill bundles) plug in here as another
/// variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbilityKind {
    /// Belongs to one registered agent; dispatch lands inside the
    /// agent's subprocess. Names: `<agent>.<verb>`.
    Agent,
    /// Belongs to the daemon (the node) itself; dispatch lands
    /// in-process via `runtime::system::*`. Names: `system.<feature>`.
    System,
}

impl AbilityKind {
    /// Infer kind from a fully-qualified ability name. Useful at
    /// dispatch-router boundaries that receive a string and need
    /// to know which sub-system owns the handler.
    pub fn from_qualified_name(name: &str) -> Self {
        if name.starts_with("system.") {
            Self::System
        } else {
            Self::Agent
        }
    }
}

/// Versions the reader will accept. When bumping:
///   1. Extend this array with the new version.
///   2. Add a migration pass that rewrites manifests from the old
///      version into the new shape on load, and re-saves them.
///   3. Only remove the old version once every supported agent
///      install has been through a migration pass.
pub const SUPPORTED_SCHEMA_VERSIONS: &[&str] = &["1"];

/// One ability the owning agent offers as a network-visible tool.
///
/// Fields
/// ------
/// * `schema_version` — on-disk schema version; `None` on read is
///   treated as the implicit v1 shape (written by a tool that
///   predated the stamping rule). Writer always stamps explicitly.
/// * `name` — the *verb* portion of the ability's name. The wire-
///   level name is assembled as `<agent>.<name>` by whoever calls
///   `qualified_name` below. Must not contain `.` (that is the
///   agent/verb separator), must not be empty.
/// * `description` — a short, human-readable blurb. Passed verbatim
///   to the tool-use contract shown to an agent choosing which tool
///   to call. Not a protocol field; safe to tune for readability.
/// * `timeout_seconds` — upper bound on how long an invocation may
///   run before the dispatcher aborts it. `None` inherits the
///   runtime default (see `support::timeouts`). The *unit* is
///   seconds specifically — we carry the raw `u64` and only convert
///   to `Duration` at the boundary so TOML round-trips are exact.
/// * `input_schema` / `output_schema` — JSON Schema documents.
///   `input_schema` is required and must be a JSON object at its
///   top level (`{"type": "object", ...}`). `output_schema` is
///   optional; absence means "the ability returns opaque content"
///   (typical for a chat-style ability whose reply is a string the
///   caller is expected to post-process).
///
/// Why the two schemas are `serde_json::Value`
/// --------------------------------------------
/// A JSON Schema is itself a tree of nested objects with
/// schema-specific keywords (`$ref`, `oneOf`, etc.). Typing that
/// tree statically would reimplement a JSON Schema crate in our
/// ontology layer; instead we carry a validated `Value` and let
/// downstream tooling (Axon's ToolSpec, OpenAI's tool-use contract)
/// validate on the read side. Our own validation is limited to
/// "top-level is an object" — the one invariant that makes every
/// consumer's failure mode the same.
///
/// Why private fields with getters
/// -------------------------------
/// Construction goes through `AbilityManifest::new` or
/// `from_toml_str`; both run `validate()`. Public fields would
/// let a caller mint a malformed manifest with a literal, which
/// would then explode in a distant consumer at read time. The
/// narrow constructor makes "well-formed by construction" the
/// only path.
// No `Eq`: `serde_json::Value` contains `f64` which is not `Eq`-able
// (NaN != NaN). `AgentSpec` has `Eq`, so the asymmetry would otherwise
// be surprising to a reader expecting `HashMap<AbilityManifest, _>` or
// `BTreeSet<AbilityManifest>` to compile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AbilityManifest {
    #[serde(skip_serializing_if = "Option::is_none")]
    schema_version: Option<String>,
    name: String,
    description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout_seconds: Option<u64>,
    input_schema: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_schema: Option<Value>,
}

impl AbilityManifest {
    /// Build a manifest, validating the fields that downstream
    /// consumers rely on.
    ///
    /// This is the canonical constructor; `from_toml_str` funnels
    /// through the same `validate()` once it has deserialized.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
    ) -> anyhow::Result<Self> {
        let m = Self {
            schema_version: Some(CURRENT_SCHEMA_VERSION.to_string()),
            name: name.into(),
            description: description.into(),
            timeout_seconds: None,
            input_schema,
            output_schema: None,
        };
        m.validate()?;
        Ok(m)
    }

    /// Override the default `None` timeout. Returns `self` for the
    /// builder-style chain a caller of `new(...)` might use.
    pub fn with_timeout_seconds(mut self, seconds: u64) -> anyhow::Result<Self> {
        self.timeout_seconds = Some(seconds);
        self.validate()?;
        Ok(self)
    }

    /// Attach an `output_schema`. Optional; only set when the
    /// ability has a typed return contract (code-review scorecard,
    /// structured evaluation, etc.) — chat-style abilities
    /// deliberately leave it absent.
    pub fn with_output_schema(mut self, schema: Value) -> anyhow::Result<Self> {
        self.output_schema = Some(schema);
        self.validate()?;
        Ok(self)
    }

    /// Parse from TOML. Validates before returning — a manifest
    /// whose disk form is well-formed TOML but semantically invalid
    /// (empty name, input_schema that isn't an object, …) becomes
    /// an error here, not a subtle bug in a downstream call site.
    pub fn from_toml_str(toml: &str) -> anyhow::Result<Self> {
        let m: Self = ::toml::from_str(toml)
            .map_err(|e| anyhow::anyhow!("failed to parse ability.toml: {e}"))?;
        m.validate()?;
        Ok(m)
    }

    /// Serialize to TOML. The writer always stamps the current
    /// schema version — even when the loaded manifest came in
    /// without one — so the round-tripped file is always
    /// forward-self-describing.
    pub fn to_toml_string(&self) -> anyhow::Result<String> {
        let mut stamped = self.clone();
        stamped.schema_version = Some(CURRENT_SCHEMA_VERSION.to_string());
        ::toml::to_string_pretty(&stamped)
            .map_err(|e| anyhow::anyhow!("failed to serialize ability.toml: {e}"))
    }

    /// The verb portion of the ability name as written on disk.
    /// Callers assembling the wire-level `<agent>.<verb>` use
    /// [`qualified_name`](Self::qualified_name) instead.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Human-readable blurb. Not a protocol field.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Per-ability invocation timeout. `None` means "inherit
    /// runtime default"; see module doc for semantics.
    pub fn timeout_seconds(&self) -> Option<u64> {
        self.timeout_seconds
    }

    /// The required input schema. Always an object at its top level
    /// (enforced by `validate()`).
    pub fn input_schema(&self) -> &Value {
        &self.input_schema
    }

    /// The optional output schema.
    pub fn output_schema(&self) -> Option<&Value> {
        self.output_schema.as_ref()
    }

    /// Build the wire-level fully-qualified name `<agent>.<verb>`.
    /// The dot is the reserved separator — the agent-name validator
    /// in `registry::agents` rejects any agent name that contains a
    /// dot, so this concatenation is unambiguous by construction.
    pub fn qualified_name(&self, agent_name: &str) -> String {
        format!("{agent_name}.{}", self.name)
    }

    /// Validate the invariants every consumer relies on.
    fn validate(&self) -> anyhow::Result<()> {
        if let Some(v) = &self.schema_version {
            if !SUPPORTED_SCHEMA_VERSIONS.contains(&v.as_str()) {
                anyhow::bail!(
                    "ability.toml schema_version = {:?} is not supported (known: {:?})",
                    v,
                    SUPPORTED_SCHEMA_VERSIONS
                );
            }
        }
        let trimmed = self.name.trim();
        if trimmed.is_empty() {
            anyhow::bail!("ability.toml `name` must not be empty");
        }
        if trimmed != self.name {
            anyhow::bail!(
                "ability.toml `name` must not contain leading/trailing whitespace: {:?}",
                self.name
            );
        }
        if self.name.contains('.') {
            anyhow::bail!(
                "ability.toml `name` must not contain `.` — that is the agent/verb \
                 separator. Got {:?}",
                self.name
            );
        }
        if self.name.contains('/') || self.name.contains(std::path::MAIN_SEPARATOR) {
            anyhow::bail!(
                "ability.toml `name` must not contain path separators: {:?}",
                self.name
            );
        }
        // Reject control characters and whitespace in the interior —
        // the name will end up in a wire-level tool identifier and
        // an embedded space would turn it into two tokens. The
        // check is deliberately strict: any non-visible-ASCII run
        // gets rejected.
        for c in self.name.chars() {
            if c.is_control() || c.is_whitespace() {
                anyhow::bail!(
                    "ability.toml `name` must not contain whitespace or control chars: \
                     {:?}",
                    self.name
                );
            }
        }
        if !self.input_schema.is_object() {
            anyhow::bail!(
                "ability.toml `input_schema` must be a JSON object at the top level \
                 (got {}); JSON Schema needs `{{\"type\": \"object\", ...}}`",
                match &self.input_schema {
                    Value::Null => "null",
                    Value::Bool(_) => "a boolean",
                    Value::Number(_) => "a number",
                    Value::String(_) => "a string",
                    Value::Array(_) => "an array",
                    Value::Object(_) => unreachable!(),
                }
            );
        }
        if let Some(out) = &self.output_schema {
            if !out.is_object() {
                anyhow::bail!(
                    "ability.toml `output_schema`, when present, must be a JSON \
                     object (JSON Schema)"
                );
            }
        }
        if let Some(0) = self.timeout_seconds {
            anyhow::bail!(
                "ability.toml `timeout_seconds` of 0 is a footgun — it means `kill \
                 immediately` to the subprocess supervisor. If you want \"inherit \
                 the runtime default\", omit the field. If you want \"no timeout\", \
                 pick a real upper bound."
            );
        }
        Ok(())
    }
}

/// Build the default `chat` manifest that every freshly-created
/// agent ships with. The agent's default input channel is surfaced
/// as a `chat` ability so external callers can reach it over the
/// network without any extra operator action.
///
/// Parity with `runtime::abilities::chat_ability`
/// ----------------------------------------------
/// Two sources of truth exist for the `chat` ability's shape until
/// a later PR collapses them: this helper (on-disk template) and
/// `runtime::abilities::chat_ability` (hardcoded baseline used by
/// today's dispatch + discovery). Only the **input_schema** is a
/// protocol contract that must match — publishing two different
/// tool specs depending on which path discovery goes through is
/// exactly the silent-fail the parity guard exists to catch.
/// **Descriptions are allowed to differ**: the hardcoded side
/// interpolates the agent name for better UX at discovery time;
/// this template is agent-agnostic because a manifest does not
/// know which agent it belongs to.
///
/// The input_schema parity is pinned by
/// `hardcoded_chat_ability_input_schema_agrees_with_default_chat_manifest`
/// in `runtime::abilities`'s test module — if you touch the shape
/// on either side, update both or the parity test will fail loud.
pub fn default_chat_manifest() -> AbilityManifest {
    // The schema below is the wire contract for the chat ability. It
    // is intentionally backward-compatible: only `prompt` is required;
    // every newer field is optional, so a legacy caller sending only
    // `{ "prompt": "...", "context": "..." }` still validates and runs
    // identically to the pre-refactor behaviour. The new fields exist
    // so the chat handler can: (1) resume a multi-turn session via
    // `session_id`, (2) decide which other abilities of the same agent
    // to expose to the LLM as tools (`skills`), (3) decide which
    // context loaders to run before invoking the LLM
    // (`context_loaders`), (4) override per-invocation driver knobs
    // without editing agent.toml (`driver`), and (5) flip on a
    // streaming RPC variant (`stream`).
    //
    // `additionalProperties: false` is load-bearing — sending an
    // unrecognised top-level field surfaces as a schema error rather
    // than silently being dropped, which makes "I added context but
    // it didn't take effect" tractable to debug. Sub-objects use the
    // same rule recursively.
    let input_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "prompt": {
                "type": "string",
                "description": "The user prompt sent to the agent."
            },
            "context": {
                "type": "string",
                "description": "Optional system-style preamble prepended before `prompt`. \
                                Carried through to compose_prompt() as a literal string; \
                                use `context_loaders` instead when the data should come \
                                from a registered loader."
            },
            "session_id": {
                "type": "string",
                "description": "Optional conversation id to resume an existing session. When \
                                omitted the chat handler creates a fresh one and returns the \
                                generated id in the response."
            },
            "skills": {
                "type": "object",
                "description": "Controls which of this agent's other abilities are exposed \
                                to the LLM as tools for the current invocation.",
                "properties": {
                    "mode": {
                        "type": "string",
                        "enum": ["auto", "none", "explicit"],
                        "description": "auto = expose every ability of this agent (default); \
                                        none = expose nothing; explicit = expose only those \
                                        listed in `include`."
                    },
                    "include": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Fully-qualified ability names (`<agent>.<verb>`) to \
                                        expose. Honoured in `explicit` mode; ignored in \
                                        `auto`/`none`."
                    },
                    "exclude": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Fully-qualified ability names to filter out, applied \
                                        after `mode`/`include`. Useful with `auto` to drop \
                                        a noisy or expensive tool from a single call."
                    }
                },
                "additionalProperties": false
            },
            "context_loaders": {
                "type": "object",
                "description": "Controls which registered context loaders run before the LLM \
                                is invoked. Each loader's output is appended to the prompt's \
                                context block, alongside the literal `context` arg if any.",
                "properties": {
                    "mode": {
                        "type": "string",
                        "enum": ["auto", "none", "explicit"]
                    },
                    "include": {
                        "type": "array",
                        "items": {"type": "string"}
                    },
                    "exclude": {
                        "type": "array",
                        "items": {"type": "string"}
                    }
                },
                "additionalProperties": false
            },
            "driver": {
                "type": "object",
                "description": "Per-invocation overrides for the underlying LLM driver. \
                                Omit to use the agent's defaults from agent.toml.",
                "properties": {
                    "model": {
                        "type": "string",
                        "description": "Override the agent's default model for this call."
                    },
                    "temperature": {
                        "type": "number",
                        "minimum": 0,
                        "maximum": 2
                    },
                    "max_tokens": {
                        "type": "integer",
                        "minimum": 1
                    }
                },
                "additionalProperties": false
            },
            "stream": {
                "type": "boolean",
                "description": "When true via the RPC entry point, the handler rejects the \
                                call and asks the caller to use the subscribe entry point \
                                instead. The streaming subscribe path emits typed frames \
                                (session/loaded/delta/tool_call_*/done|error)."
            }
        },
        "required": ["prompt"],
        "additionalProperties": false,
    });

    // Output schema documents what an RPC invocation returns. It is
    // optional in AbilityManifest and historically the chat ability
    // omitted it (chat replies were "opaque text"). With the refactor
    // we publish a typed shape so the EasyNet frontend's ability
    // detail card can render structured output, and so an agent
    // composing other abilities can introspect what to expect.
    //
    // Most fields are required because they appear on every RPC reply
    // — `usage` is the exception (LLM driver may not surface token
    // counts on every backend).
    let output_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "session_id": {
                "type": "string",
                "description": "The session id used for this turn. Echoes the input when \
                                provided; freshly generated otherwise."
            },
            "reply": {
                "type": "string",
                "description": "The LLM's final reply text. The legacy single-string return \
                                value lives here; pre-refactor callers can read just this \
                                field and ignore everything else."
            },
            "skills_loaded": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Fully-qualified ability names that were actually exposed to \
                                the LLM as tools for this call (after applying `skills.mode` \
                                and `exclude`)."
            },
            "tool_calls": {
                "type": "array",
                "description": "Per-tool-call observability: every ability the LLM invoked \
                                during this turn, in order, with args/result/error/elapsed.",
                "items": {
                    "type": "object",
                    "properties": {
                        "ability": {"type": "string"},
                        "args": {},
                        "result": {},
                        "error": {"type": "string"},
                        "elapsed_ms": {"type": "integer", "minimum": 0}
                    },
                    "required": ["ability", "elapsed_ms"]
                }
            },
            "context_used": {
                "type": "array",
                "description": "Per-loader contribution: which context loaders ran and how \
                                many bytes each contributed to the assembled context block.",
                "items": {
                    "type": "object",
                    "properties": {
                        "loader": {"type": "string"},
                        "bytes": {"type": "integer", "minimum": 0}
                    },
                    "required": ["loader", "bytes"]
                }
            },
            "usage": {
                "type": "object",
                "description": "Token accounting reported by the driver, when the underlying \
                                LLM backend exposes it.",
                "properties": {
                    "input_tokens": {"type": "integer", "minimum": 0},
                    "output_tokens": {"type": "integer", "minimum": 0},
                    "model": {"type": "string"}
                }
            },
            "elapsed_ms": {
                "type": "integer",
                "minimum": 0,
                "description": "Wall-clock duration of the full chat invocation."
            }
        },
        "required": ["session_id", "reply", "skills_loaded", "tool_calls", "context_used", "elapsed_ms"]
    });

    AbilityManifest::new(
        "chat",
        "Send a chat prompt to the locally-installed agent. The agent runs as a \
         subprocess on this node; the response is returned verbatim. The optional \
         `skills`, `context_loaders`, and `driver` sub-objects let a single call \
         override skill exposure, context assembly, and driver knobs without \
         editing the agent's manifest.",
        input_schema,
    )
    .expect(
        "default_chat_manifest is a constant, well-formed input; validation failing \
         here would be a compile-time contract violation in this file",
    )
    .with_output_schema(output_schema)
    .expect(
        "the embedded output schema is a JSON object; validation failure here would \
         be a compile-time contract violation in this file",
    )
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! The tests pin the invariants every consumer relies on:
    //! construction-time validation, stamped schema version on
    //! write, qualified-name assembly, and TOML round-trip.

    use super::*;
    use serde_json::json;

    fn object_schema() -> Value {
        json!({"type": "object", "properties": {}, "required": []})
    }

    // ── happy path ──────────────────────────────────────────────────────────

    #[test]
    fn new_builds_and_validates_a_minimal_manifest() {
        let m = AbilityManifest::new("chat", "hello", object_schema()).unwrap();
        assert_eq!(m.name(), "chat");
        assert_eq!(m.description(), "hello");
        assert!(m.input_schema().is_object());
        assert!(m.output_schema().is_none());
        assert_eq!(m.timeout_seconds(), None);
    }

    #[test]
    fn qualified_name_concatenates_agent_and_verb_with_dot() {
        let m = AbilityManifest::new("chat", "x", object_schema()).unwrap();
        assert_eq!(m.qualified_name("alice"), "alice.chat");
    }

    #[test]
    fn default_chat_manifest_matches_hardcoded_baseline_shape() {
        // Guards against the default-manifest helper drifting away
        // from the runtime::abilities baseline before the two paths
        // converge in a later PR. If this breaks, update both
        // sides in the same PR, not one at a time.
        let m = default_chat_manifest();
        assert_eq!(m.name(), "chat");
        let props = m
            .input_schema()
            .get("properties")
            .and_then(Value::as_object)
            .expect("properties must be an object");
        assert!(props.contains_key("prompt"));
        assert!(props.contains_key("context"));
        let required = m
            .input_schema()
            .get("required")
            .and_then(Value::as_array)
            .expect("required is an array");
        assert!(required.iter().any(|v| v.as_str() == Some("prompt")));
        assert_eq!(
            m.input_schema().get("additionalProperties"),
            Some(&Value::Bool(false)),
            "schema must reject extra args"
        );
    }

    #[test]
    fn default_chat_manifest_declares_extended_input_fields() {
        // The post-refactor input schema adds optional fields that the
        // chat handler reads at invocation time. Pin every one — losing
        // any of them silently would break the contract the EasyNet
        // backend's ability detail card depends on.
        let m = default_chat_manifest();
        let props = m
            .input_schema()
            .get("properties")
            .and_then(Value::as_object)
            .expect("properties must be an object");
        for required_key in [
            "prompt",
            "context",
            "session_id",
            "skills",
            "context_loaders",
            "driver",
            "stream",
        ] {
            assert!(
                props.contains_key(required_key),
                "input_schema.properties is missing {required_key:?}; got keys = {:?}",
                props.keys().collect::<Vec<_>>()
            );
        }
        // Only `prompt` is required — every newer field is optional so
        // a legacy `{"prompt": "..."}` call still validates.
        let required = m
            .input_schema()
            .get("required")
            .and_then(Value::as_array)
            .expect("required is an array");
        assert_eq!(required.len(), 1);
        assert_eq!(required[0].as_str(), Some("prompt"));
    }

    #[test]
    fn default_chat_manifest_skills_subobject_uses_mode_include_exclude() {
        // The shape `{ mode, include, exclude }` is shared between
        // `skills` and `context_loaders` so a renderer (frontend
        // SchemaForm) can treat them uniformly. Drift on either side
        // would break that uniformity.
        let m = default_chat_manifest();
        for key in ["skills", "context_loaders"] {
            let sub = m
                .input_schema()
                .get("properties")
                .and_then(|p| p.get(key))
                .and_then(|s| s.get("properties"))
                .and_then(Value::as_object)
                .unwrap_or_else(|| panic!("{key} sub-object must declare properties"));
            for inner in ["mode", "include", "exclude"] {
                assert!(
                    sub.contains_key(inner),
                    "{key}.{inner} missing; got {:?}",
                    sub.keys().collect::<Vec<_>>()
                );
            }
            let mode_enum = m
                .input_schema()
                .get("properties")
                .and_then(|p| p.get(key))
                .and_then(|s| s.get("properties"))
                .and_then(|p| p.get("mode"))
                .and_then(|m| m.get("enum"))
                .and_then(Value::as_array)
                .unwrap_or_else(|| panic!("{key}.mode.enum must be a list"));
            let modes: Vec<&str> = mode_enum.iter().filter_map(Value::as_str).collect();
            assert_eq!(modes, vec!["auto", "none", "explicit"]);
        }
    }

    #[test]
    fn default_chat_manifest_publishes_typed_output_schema() {
        // Pre-refactor chat omitted output_schema (opaque text). Post-
        // refactor we publish a typed shape so the EasyNet ability
        // detail card can render structured output and so an agent
        // composing other abilities knows what to expect.
        let m = default_chat_manifest();
        let out = m
            .output_schema()
            .expect("default chat manifest must publish an output_schema");
        let props = out
            .get("properties")
            .and_then(Value::as_object)
            .expect("output_schema.properties must be an object");
        for key in [
            "session_id",
            "reply",
            "skills_loaded",
            "tool_calls",
            "context_used",
            "usage",
            "elapsed_ms",
        ] {
            assert!(
                props.contains_key(key),
                "output_schema is missing {key:?}; got {:?}",
                props.keys().collect::<Vec<_>>()
            );
        }
        // `reply` is the legacy single-string return value; pre-
        // refactor callers reading just this field must still work.
        assert_eq!(
            props
                .get("reply")
                .and_then(|p| p.get("type"))
                .and_then(Value::as_str),
            Some("string"),
        );
        // `usage` is intentionally NOT required (some drivers don't
        // surface tokens). Pin that explicitly.
        let required = out
            .get("required")
            .and_then(Value::as_array)
            .expect("required is an array");
        let req: Vec<&str> = required.iter().filter_map(Value::as_str).collect();
        assert!(!req.contains(&"usage"), "usage must NOT be required");
        for must in [
            "session_id",
            "reply",
            "skills_loaded",
            "tool_calls",
            "context_used",
            "elapsed_ms",
        ] {
            assert!(req.contains(&must), "output_schema.required missing {must}");
        }
    }

    #[test]
    fn default_chat_manifest_rejects_unknown_top_level_args() {
        // additionalProperties: false is load-bearing. A legacy caller
        // sending only {prompt, context} validates; an unknown field
        // surfaces as a schema error rather than being silently
        // dropped, which is what makes "I added X but it didn't take
        // effect" tractable to debug.
        let m = default_chat_manifest();
        assert_eq!(
            m.input_schema().get("additionalProperties"),
            Some(&Value::Bool(false))
        );
    }

    #[test]
    fn toml_round_trip_preserves_fields() {
        let m = AbilityManifest::new("chat", "blurb", object_schema())
            .unwrap()
            .with_timeout_seconds(30)
            .unwrap()
            .with_output_schema(json!({"type": "object"}))
            .unwrap();
        let toml = m.to_toml_string().unwrap();
        let parsed = AbilityManifest::from_toml_str(&toml).unwrap();
        assert_eq!(parsed, m);
    }

    #[test]
    fn to_toml_string_always_stamps_current_schema_version() {
        // Even if the in-memory manifest has schema_version=None
        // (shape a legacy file might produce), the writer stamps
        // the current version. Downstream readers rely on this
        // for forward self-description.
        let mut m = AbilityManifest::new("chat", "x", object_schema()).unwrap();
        m.schema_version = None;
        let toml = m.to_toml_string().unwrap();
        assert!(
            toml.contains(&format!("schema_version = \"{CURRENT_SCHEMA_VERSION}\"")),
            "writer must stamp CURRENT_SCHEMA_VERSION; got:\n{toml}"
        );
    }

    #[test]
    fn from_toml_str_accepts_missing_schema_version_as_v1_shape() {
        // A manifest written before the stamping rule landed has
        // no schema_version. We read it as implicit v1 and do not
        // fail — otherwise every pre-existing dev install would
        // break on upgrade.
        let toml = format!(
            "name = \"chat\"\n\
             description = \"x\"\n\
             [input_schema]\n\
             type = \"object\"\n"
        );
        let m = AbilityManifest::from_toml_str(&toml).unwrap();
        assert_eq!(m.name(), "chat");
    }

    // ── failure path ────────────────────────────────────────────────────────

    #[test]
    fn new_rejects_empty_name() {
        let err = AbilityManifest::new("", "x", object_schema()).unwrap_err();
        assert!(format!("{err}").contains("must not be empty"));
    }

    #[test]
    fn new_rejects_whitespace_only_name() {
        let err = AbilityManifest::new("   ", "x", object_schema()).unwrap_err();
        assert!(format!("{err}").contains("empty"));
    }

    #[test]
    fn new_rejects_name_containing_dot() {
        // `.` is the agent/verb separator; embedding one would
        // make `<agent>.<name>` ambiguous on the wire.
        let err = AbilityManifest::new("chat.v2", "x", object_schema()).unwrap_err();
        assert!(format!("{err}").contains("`."));
    }

    #[test]
    fn new_rejects_name_with_slash() {
        let err = AbilityManifest::new("my/chat", "x", object_schema()).unwrap_err();
        assert!(format!("{err}").contains("path separators"));
    }

    #[test]
    fn new_rejects_name_with_interior_whitespace() {
        let err = AbilityManifest::new("my chat", "x", object_schema()).unwrap_err();
        assert!(format!("{err}").contains("whitespace"));
    }

    #[test]
    fn new_rejects_name_with_control_character() {
        let err = AbilityManifest::new("chat\t", "x", object_schema()).unwrap_err();
        assert!(
            format!("{err}").contains("whitespace") || format!("{err}").contains("control")
        );
    }

    #[test]
    fn new_rejects_input_schema_that_is_not_an_object() {
        let err = AbilityManifest::new("chat", "x", json!(["a", "b"])).unwrap_err();
        assert!(format!("{err}").contains("object"));
    }

    #[test]
    fn new_rejects_input_schema_null() {
        let err = AbilityManifest::new("chat", "x", json!(null)).unwrap_err();
        assert!(format!("{err}").contains("null"));
    }

    #[test]
    fn with_output_schema_rejects_non_object() {
        let base = AbilityManifest::new("chat", "x", object_schema()).unwrap();
        let err = base.with_output_schema(json!(42)).unwrap_err();
        assert!(format!("{err}").contains("object"));
    }

    #[test]
    fn with_timeout_seconds_rejects_zero() {
        // Zero-timeout means "kill immediately" to the supervisor;
        // the field is for upper bounds, not abort switches. A
        // user wanting the default should omit the field, not
        // write 0.
        let base = AbilityManifest::new("chat", "x", object_schema()).unwrap();
        let err = base.with_timeout_seconds(0).unwrap_err();
        assert!(format!("{err}").contains("0"));
    }

    #[test]
    fn from_toml_str_rejects_malformed_toml() {
        let err = AbilityManifest::from_toml_str("not = a = valid = toml").unwrap_err();
        assert!(format!("{err}").contains("parse"));
    }

    #[test]
    fn from_toml_str_rejects_unknown_schema_version() {
        // Forward-compat: a writer that stamps an unknown version
        // (99) must be rejected loudly, not accepted with a silent
        // "pretend it's v1" fallback.
        let toml = "schema_version = \"99\"\n\
                    name = \"chat\"\n\
                    description = \"x\"\n\
                    [input_schema]\n\
                    type = \"object\"\n";
        let err = AbilityManifest::from_toml_str(toml).unwrap_err();
        assert!(format!("{err}").contains("schema_version"));
    }

    // ── edge cases ──────────────────────────────────────────────────────────

    #[test]
    fn name_with_dashes_and_underscores_and_digits_is_accepted() {
        // These are the allowed character class for verb names.
        // The test exists so the validator change log has to
        // argue with a pinned contract rather than silently shift
        // the allowed set.
        for name in ["chat", "chat_v2", "chat-v2", "chat2"] {
            AbilityManifest::new(name, "x", object_schema())
                .unwrap_or_else(|e| panic!("{name:?} should be accepted: {e}"));
        }
    }

    #[test]
    fn empty_description_is_accepted_even_though_it_is_bad_ux() {
        // We don't gatekeep description — the UX layer can warn,
        // but the protocol layer should not refuse to load.
        // Rejecting here would block an operator from committing
        // a WIP manifest.
        let m = AbilityManifest::new("chat", "", object_schema()).unwrap();
        assert_eq!(m.description(), "");
    }

    #[test]
    fn large_timeout_seconds_round_trips_exactly() {
        // 24h * 7 * 365 as a sanity bound. We carry u64 to keep
        // the door open for long-running batch abilities without
        // having to widen the type later.
        let secs: u64 = 60 * 60 * 24 * 7 * 365;
        let m = AbilityManifest::new("chat", "x", object_schema())
            .unwrap()
            .with_timeout_seconds(secs)
            .unwrap();
        let toml = m.to_toml_string().unwrap();
        let parsed = AbilityManifest::from_toml_str(&toml).unwrap();
        assert_eq!(parsed.timeout_seconds(), Some(secs));
    }

    #[test]
    fn input_schema_with_nested_references_round_trips() {
        // A realistic JSON Schema uses `$ref` and `oneOf` etc.
        // We don't validate those — we just ensure the
        // serde-json passthrough survives.
        let schema = json!({
            "type": "object",
            "properties": {
                "prompt": {"type": "string"},
                "tools": {
                    "type": "array",
                    "items": {"$ref": "#/definitions/Tool"}
                }
            },
            "required": ["prompt"],
            "definitions": {
                "Tool": {
                    "oneOf": [
                        {"const": "shell"},
                        {"const": "edit"}
                    ]
                }
            }
        });
        let m = AbilityManifest::new("chat", "x", schema).unwrap();
        let toml = m.to_toml_string().unwrap();
        let parsed = AbilityManifest::from_toml_str(&toml).unwrap();
        assert_eq!(parsed, m);
    }

    #[test]
    fn qualified_name_with_unicode_agent_is_stored_verbatim() {
        // We do not re-validate the agent name here — it has
        // already been validated by `registry::agents` upstream.
        // The manifest's only job is to concatenate.
        let m = AbilityManifest::new("chat", "x", object_schema()).unwrap();
        assert_eq!(m.qualified_name("alice"), "alice.chat");
    }
}
