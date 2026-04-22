// EasyNet CLI — Runtime Abilities (default-chat surface)
// =============================================================
//
// File: src/runtime/abilities.rs
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
// EasyNet-Axon's SDK distinguishes two paths for "making something
// callable from the network":
//
//   - `DendriteBridge::publish_capability(...)` — for a distributable
//     *package* (tar.gz, signed) that the runtime will install and
//     execute on a node. Semantically static: the payload is a
//     serializable artifact that can be replicated between nodes.
//
//   - `AbilityToolAdapter::register(name, handler, spec)` — for a
//     *live handler* that lives on *this* node only. Incoming RPC
//     calls against the adapter dispatch directly to a local Rust
//     closure; the capability is advertised via node labels
//     (`a2a.agents_json[*].skills`) so discovery finds it.
//
// A locally-installed AI agent (Claude Code, Codex, …) is not a
// distributable package — it is a subprocess binding that only
// makes sense on the node where the operator installed it. The
// adapter path is therefore the semantically correct one.
//
// This module is the neutral ground between the registry
// (`~/.easynet/agents.json`, which records what the operator has
// installed locally) and the adapter wiring (`facade::mcp::agent_dispatch`,
// which plumbs those records into incoming MCP tool calls). It
// answers exactly one question — "for agent X of type Y, what
// abilities does it offer as network-visible tools, and what is
// the JSON schema of each ability's arguments" — and answers it
// the same way from two call sites:
//
//   1. `registry::a2a_labels::build` — to include in `a2a.agents_json[*].skills`
//      so federated peers *discover* the abilities without calling
//      anything.
//   2. `facade::mcp::agent_dispatch::AgentDispatchAdapter::build` — to register
//      each ability as a local tool on the `AbilityToolAdapter`,
//      so the same ability *can be invoked*.
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

use serde_json::{json, Value};

use crate::registry::agents::AgentEntry;

/// One ability exposed by a locally-installed agent.
///
/// The three fields are the intersection of what every consumer of
/// this spec needs:
///
///   * `name`      — the tool name as it will appear on the MCP wire
///                   and in `a2a.agents_json[*].skills[*].name`. Must
///                   obey the `<agent>.<verb>` shape and be unique
///                   within a single agent's ability list.
///   * `description` — one-sentence human-readable blurb, surfaced
///                   by agent-side tool pickers when the agent is
///                   choosing which tool to call. Not a protocol
///                   field; may be tuned for readability.
///   * `parameters` — JSON Schema describing the argument shape.
///                   Must be a JSON object at the top level
///                   (`{"type": "object", ...}`); that is the only
///                   shape both OpenAI's tool-use contract and the
///                   Axon SDK's ToolSpec accept.
///
/// Private fields with public getters: callers construct specs via
/// `AgentAbilitySpec::new(...)` (which validates the shape) or the
/// module-level `abilities_for(...)`, never by field-wise struct
/// literals. This keeps every instance well-formed by construction
/// and makes "I'll just set description = empty" a compile error at
/// the site, not a runtime regression downstream.
#[derive(Debug, Clone)]
pub struct AgentAbilitySpec {
    name: String,
    description: String,
    parameters: Value,
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
            parameters,
        })
    }

    /// The network-visible tool name. Stable; the identity under which
    /// this ability is both advertised (in `a2a.agents_json`) and
    /// dispatched (by `AbilityToolAdapter`).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Human-readable description. Safe to mutate across releases;
    /// not a protocol field.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The JSON Schema for this ability's arguments. Always a JSON
    /// object at the top level (enforced by `AgentAbilitySpec::new`).
    pub fn parameters(&self) -> &Value {
        &self.parameters
    }

    /// Serialise into the per-entry JSON object used inside
    /// `a2a.agents_json[*].skills`. The shape is a wire contract
    /// specified by `docs/spec/node-roster-label-v2.md` — the field
    /// names match what the EasyNet backend's `ParseAgentsJSON`
    /// reads at the v2 envelope's skill layer.
    ///
    /// Optional fields (`output_schema`, `timeout_seconds`) are
    /// emitted as explicit `null` rather than omitted. Spec §"null
    /// vs absent" fixes this writer-side rule so `golden.json` is
    /// byte-stable across rebuilds; readers on the backend tolerate
    /// either null or absent per the same spec section.
    pub fn to_discovery_json(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "input_schema": self.parameters,
            "output_schema": Value::Null,
            "timeout_seconds": Value::Null,
        })
    }
}

/// Build the ability list for one agent entry.
///
/// Today this returns a single `<agent>.chat` ability with the
/// schema `{ prompt: string, context?: string }`. `context` is
/// optional because the incoming remote caller may or may not want
/// to stitch a system prefix — either is honest for an external
/// invocation.
///
/// Adding a second ability (e.g. `voice`, `exec`) is a matter of
/// pushing a new spec into the returned Vec and — if its semantics
/// depend on the agent type — branching on `entry.agent_type()`.
/// Consumers iterate the `Vec`, so a new ability automatically
/// appears in discovery and dispatch simultaneously.
pub fn abilities_for(agent_name: &str, _entry: &AgentEntry) -> Vec<AgentAbilitySpec> {
    // The unused `_entry` parameter is intentional: today every agent
    // type offers the same single ability, so we ignore the type tag,
    // but keeping the parameter in the signature makes per-type
    // branching trivial to add without a second-pass API change. The
    // underscore silences the unused-variable lint without claiming
    // the parameter is dead — a future per-type branch will inspect
    // `entry.agent_type()` here.
    vec![chat_ability(agent_name).expect(
        "chat_ability produces a well-formed spec for any validated agent name \
         (validate_agent_name guarantees the `<agent>.chat` shape is legal)",
    )]
}

/// Build the `<agent>.chat` spec for a given agent name.
///
/// Factored out so tests can exercise just the shape of the chat
/// spec — and so a hypothetical future "abilities differ by type"
/// variant does not have to open-code the chat spec again.
fn chat_ability(agent_name: &str) -> Result<AgentAbilitySpec, &'static str> {
    let name = format!("{agent_name}.chat");
    let description = format!(
        "Send a chat prompt to the locally-installed `{agent_name}` agent. \
         The agent runs as a subprocess on this node; the response is returned \
         verbatim. Use `context` to prepend a system-style preamble when the \
         agent supports one."
    );
    let parameters = json!({
        "type": "object",
        "properties": {
            "prompt": {
                "type": "string",
                "description": "The user prompt sent to the agent."
            },
            "context": {
                "type": "string",
                "description": "Optional system-style preamble prepended before `prompt`."
            },
        },
        "required": ["prompt"],
        "additionalProperties": false,
    });
    AgentAbilitySpec::new(name, description, parameters)
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! Tests aim to guard the invariants the module's docstrings
    //! promise — not just "it builds a spec". Each test names the
    //! invariant in its function name so a failure report tells the
    //! operator which promise just broke.

    use super::*;
    use crate::registry::agents::AgentType;

    fn entry_of(t: AgentType) -> AgentEntry {
        AgentEntry::new(t, None)
    }

    #[test]
    fn abilities_for_returns_exactly_one_chat_spec_per_agent_type() {
        // The current contract is "one ability per agent, named
        // `<agent>.chat`". Pinning the count here is what catches a
        // future "let me silently add a second ability" patch that
        // would break downstream consumers which assume a one-to-one
        // agent↔ability mapping.
        for t in [
            AgentType::ClaudeCode,
            AgentType::Codex,
            AgentType::CodexAppServer,
        ] {
            let entry = entry_of(t);
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
    fn chat_ability_schema_is_object_with_required_prompt() {
        let abilities = abilities_for("claude", &entry_of(AgentType::ClaudeCode));
        let params = abilities[0].parameters();
        // Top-level type is an object — every LLM tool-use contract
        // OpenAI / Anthropic / Axon emits assumes this. Verify it
        // here so a future "let me simplify the schema" patch can't
        // silently break tool registration at runtime.
        assert_eq!(
            params.get("type").and_then(Value::as_str),
            Some("object"),
            "schema top-level type must be \"object\""
        );
        let required = params
            .get("required")
            .and_then(Value::as_array)
            .expect("required must be an array");
        assert!(
            required.iter().any(|v| v.as_str() == Some("prompt")),
            "prompt must appear in `required`"
        );
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
    fn chat_ability_parameters_declare_both_prompt_and_optional_context() {
        let abilities = abilities_for("codex", &entry_of(AgentType::Codex));
        let props = abilities[0]
            .parameters()
            .get("properties")
            .and_then(Value::as_object)
            .expect("properties must be an object");
        assert!(props.contains_key("prompt"));
        assert!(props.contains_key("context"));
        assert_eq!(
            props.get("prompt").and_then(|p| p.get("type")).and_then(Value::as_str),
            Some("string"),
        );
        assert_eq!(
            props.get("context").and_then(|p| p.get("type")).and_then(Value::as_str),
            Some("string"),
        );
    }

    #[test]
    fn to_discovery_json_shape_is_pinned() {
        // The discovery shape is a wire contract parsed by the EasyNet
        // backend. It is specified by `docs/spec/node-roster-label-v2.md`.
        // If this test breaks, the backend's companion parser must be
        // updated in the same release window and
        // `tests/fixtures/a2a-v2/golden.json` re-generated.
        let spec = abilities_for("claude", &entry_of(AgentType::ClaudeCode))
            .into_iter()
            .next()
            .unwrap();
        let json = spec.to_discovery_json();
        let obj = json.as_object().expect("discovery json is an object");
        // Five keys: name, description, input_schema, output_schema,
        // timeout_seconds. The last two are `null` for the seeded chat
        // ability but MUST be present — the writer chose `null` over
        // absent so the golden fixture stays byte-stable.
        assert_eq!(
            obj.len(),
            5,
            "exactly 5 keys: name, description, input_schema, output_schema, timeout_seconds — got {obj:?}"
        );
        assert!(obj.contains_key("name"));
        assert!(obj.contains_key("description"));
        assert!(obj.contains_key("input_schema"));
        assert!(obj.contains_key("output_schema"));
        assert!(obj.contains_key("timeout_seconds"));
        assert!(
            obj["input_schema"].is_object(),
            "input_schema must be a JSON object (JSON Schema shape)"
        );
        assert!(
            obj["output_schema"].is_null(),
            "output_schema is null for the seeded chat ability"
        );
        assert!(
            obj["timeout_seconds"].is_null(),
            "timeout_seconds is null for the seeded chat ability"
        );
    }

    #[test]
    fn different_agent_names_produce_distinct_ability_names() {
        // Sanity: the `<agent>.chat` template must interpolate the
        // right name. A broken template that hardcoded "agent.chat"
        // would silently alias every agent to the same tool name on
        // the wire, which would make discovery and dispatch ambiguous.
        let a = abilities_for("claude", &entry_of(AgentType::ClaudeCode));
        let b = abilities_for("codex", &entry_of(AgentType::Codex));
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
        let abilities = abilities_for("claude", &entry_of(AgentType::ClaudeCode));
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
        let err =
            AgentAbilitySpec::new("   ", "desc", json!({"type": "object"})).expect_err("should err");
        assert!(err.contains("empty"));
    }

    #[test]
    fn new_rejects_dotless_name() {
        let err =
            AgentAbilitySpec::new("chat", "desc", json!({"type": "object"})).expect_err("should err");
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
        assert!(spec.parameters().is_object());
    }

    /// Regression: `AgentAbilitySpec` must be `Clone` so a discovery
    /// consumer can snapshot the spec list without holding a borrow
    /// on the registry. Compile-time check via a noop round-trip.
    #[test]
    fn spec_is_cloneable() {
        let spec = abilities_for("claude", &entry_of(AgentType::ClaudeCode))
            .into_iter()
            .next()
            .unwrap();
        let _copy = spec.clone();
    }

    /// Parity guard. Two sources of truth for the `chat` ability shape
    /// exist in this repo until a later PR collapses them:
    ///
    ///   1. `runtime::abilities::chat_ability` — the hardcoded
    ///      baseline used by today's `AgentDispatchAdapter` and
    ///      `a2a_labels::build`.
    ///   2. `core::ability_spec::default_chat_manifest` — the
    ///      on-disk template that `AgentDirectory::create` seeds
    ///      into `abilities/chat.ability.toml`.
    ///
    /// Only the input schema is a protocol contract. Descriptions
    /// differ on purpose — the hardcoded side interpolates the
    /// agent name for better UX at discovery time; the on-disk
    /// template is agent-agnostic because the manifest does not
    /// know which agent it belongs to. A divergence on the *input
    /// schema* would publish different tool specs depending on
    /// which path discovery/dispatch goes through — that is the
    /// silent-fail class this test exists to catch.
    #[test]
    fn hardcoded_chat_ability_input_schema_agrees_with_default_chat_manifest() {
        use crate::core::ability_spec::default_chat_manifest;
        let manifest = default_chat_manifest();
        let hardcoded = chat_ability("alice").unwrap();
        assert_eq!(
            manifest.input_schema(),
            hardcoded.parameters(),
            "default_chat_manifest's input_schema must equal \
             runtime::abilities::chat_ability's parameters — they are the two \
             sources of truth for the `chat` ability's wire shape and a drift \
             would publish different tool specs via discovery vs. via \
             abilities/chat.ability.toml."
        );
    }
}
