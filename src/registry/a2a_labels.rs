// EasyNet CLI — A2A Discovery Labels
// ===================================
//
// File: src/shared/a2a_labels.rs
// Description: Single source of truth for the `a2a.*` label contract that
//              this node publishes when it registers with the Axon runtime.
//
// Why this module exists:
//   The A2A (agent-to-agent) discovery protocol reserves the `a2a.*` label
//   prefix on NodeDescriptor.labels. The *client-side encoding* of that
//   prefix — which keys we emit, what JSON shape `a2a.agents_json` carries,
//   how we describe the node in `a2a.description` — used to live inline
//   inside `cli::start::run_device_mode`. That made the encoding invisible
//   from anywhere else, untestable without a live runtime, and prone to
//   drift when the protocol evolved.
//
// Scope:
//   - Pure data. No I/O, no runtime dependencies. Takes a reference to the
//     local AgentRegistry and emits a `HashMap` suitable for dropping into
//     `DendriteBridge::register_node_with_options`.
//   - Empty registry → `None`, NOT `Some({})`. The distinction is wire-level:
//     `RegisterNodeOptions::labels = None` omits the `labels` field from the
//     RPC; `Some(empty_map)` would publish an explicit `"labels": {}`. The
//     two are not equivalent on every server, and even when they are,
//     publishing a sentinel "I have zero labels" is more confusing than
//     sending nothing. The type reflects the truth — callers don't need
//     to guard `is_empty()` themselves.
//
// Reserved keys (must match the server-side `NodeDescriptor.labels` schema):
//   a2a.version       = schema version this node's labels conform to
//   a2a.enabled       = "true"
//   a2a.name          = <device hostname>
//   a2a.agents_json   = JSON array, one object per registered agent
//   a2a.description   = human-readable summary line
//
// Forward compatibility:
//   `a2a.version` is stamped on every registration so the Hub (and
//   federated peers) can distinguish clients emitting different schema
//   revisions. Absent-version means "pre-stamp" and should be read as
//   v1 by tolerant consumers. Bump `A2A_LABEL_SCHEMA_VERSION` whenever
//   the wire shape of `a2a.agents_json` or the reserved-key set changes
//   in a way old consumers can't parse.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::HashMap;

use serde_json::json;

use super::agents::AgentRegistry;

/// Schema version stamped inside each agent entry as
/// `a2a_schema_version`. Per `docs/spec/node-roster-label-v2.md`, the
/// version lives **per agent entry**, not as a label-level key. The
/// label-level `a2a.version` of earlier drafts is gone; rename an entry
/// here together with a companion backend PR or the backend rejects
/// with a version-mismatch error.
///
/// Version history
/// ---------------
///
/// - `"1"` — legacy shape (bare array, `type`/`parameters`/label-level
///   `a2a.version`). Emitted by CLI ≤ 1.6.x. Not produced by this
///   build; the EasyNet backend's v2-only parser rejects it.
/// - `"v2"` — **current**. Envelope `{"agents": [...]}`, per-entry
///   `a2a_schema_version`, `runtime`/`input_schema` field names,
///   optional `output_schema` and `timeout_seconds` present as
///   explicit `null` on the seeded chat ability. Spec:
///   `docs/spec/node-roster-label-v2.md`.
pub const A2A_SCHEMA_VERSION: &str = "v2";

/// Build the v2 `{"agents": [...]}` envelope as a structured Value.
/// This is the lower-level half of `build`: it produces the JSON shape
/// that downstream consumers (the `a2a.agents_json` label, the
/// `a2a.bridge.list_skills` ability handler) parse, without the
/// label-map wrapping or the size-limit warning that `build` adds for
/// the on-the-wire label encoding.
///
/// Iteration order: agent-name (BTreeMap), then per-agent skills
/// sorted by `name`. Both orderings feed the byte-stable
/// `tests/fixtures/a2a-v2/golden.json` fixture; do not switch to a
/// `HashMap` here.
pub fn build_agents_envelope(registry: &AgentRegistry) -> serde_json::Value {
    let agents_json: Vec<serde_json::Value> = registry
        .agents
        .iter()
        .map(|(name, e)| {
            let mut skills: Vec<serde_json::Value> =
                crate::runtime::abilities::abilities_for(name, e)
                    .iter()
                    .map(|spec| spec.to_discovery_json())
                    .collect();
            skills.sort_by(|a, b| {
                a.get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .cmp(b.get("name").and_then(|v| v.as_str()).unwrap_or(""))
            });
            // Optional fields carry explicit `null` rather than being
            // omitted. Spec §"null vs absent" fixes the writer rule;
            // fixture byte-stability depends on it.
            let description: serde_json::Value = e
                .label
                .clone()
                .map(serde_json::Value::String)
                .unwrap_or(serde_json::Value::Null);
            let model: serde_json::Value = e
                .model
                .clone()
                .map(serde_json::Value::String)
                .unwrap_or(serde_json::Value::Null);
            json!({
                "a2a_schema_version": A2A_SCHEMA_VERSION,
                "description": description,
                "model": model,
                "name": name,
                "runtime": e.agent_type.to_string(),
                "skills": skills,
            })
        })
        .collect();
    json!({ "agents": agents_json })
}

/// Build the `a2a.*` label map for registering this node with the Axon
/// runtime. Returns `None` — not an empty map — when the registry carries
/// no agents, so the caller can feed the result straight into
/// `RegisterNodeOptions::labels` without a redundant `is_empty()` guard.
/// See the module-level docstring for why the empty case is `None`.
pub fn build(registry: &AgentRegistry, hostname: &str) -> Option<HashMap<String, String>> {
    if registry.agents.is_empty() {
        return None;
    }

    let mut labels = HashMap::new();
    labels.insert("a2a.enabled".into(), "true".into());
    labels.insert("a2a.name".into(), hostname.to_string());

    // Build the v2 envelope `{"agents": [...]}`. The schema version
    // is stamped per-entry as `a2a_schema_version: "v2"` (spec:
    // `docs/spec/node-roster-label-v2.md` §Agent entry). There is
    // no longer a label-level `a2a.version` key — the EasyNet
    // backend's v2-only parser would ignore it.
    //
    // Agents + skills are sorted by name to keep byte output stable
    // across rebuilds — the backend CI asserts
    // `tests/fixtures/a2a-v2/golden.json` byte-equality, and that
    // rule depends on the ordering. `BTreeMap` iteration already
    // gives agent-name order; the skills vec comes pre-built from
    // `abilities_for`, but we sort defensively so a future
    // per-type branch in `abilities_for` that returned mixed order
    // would not silently break the fixture.
    //
    // `serde_json::to_string` can only fail on NaN/Infinity numbers
    // or non-string map keys; our values are strings, null, or u64,
    // so the fail branch is structurally unreachable. We
    // `debug_assert!` to surface a regression in dev and fall back
    // to an empty envelope in release (the node stays registered
    // without the roster).
    let envelope = build_agents_envelope(registry);
    let agents_json_str = match serde_json::to_string(&envelope) {
        Ok(s) => s,
        Err(_e) => {
            debug_assert!(
                false,
                "a2a.agents_json serialization failed — our shape cannot produce NaN/Infinity or non-string keys; a serde_json behavior change must have broken the invariant",
            );
            r#"{"agents":[]}"#.to_string()
        }
    };
    // Soft-limit warning at 24 KiB; the backend rejects at 32 KiB
    // (spec §"Size / ordering rules"). Surface the approach to the
    // limit on stderr so the operator sees it before the hard limit
    // bites.
    if agents_json_str.len() > 24 * 1024 {
        eprintln!(
            "warning: a2a.agents_json is {} bytes, approaching the 32 KiB node-label limit. \
             Consider splitting agents across multiple devices.",
            agents_json_str.len()
        );
    }
    labels.insert("a2a.agents_json".into(), agents_json_str);

    // RFC-001 P2.4: a2a.system_skills_json label retired.
    //
    // Per RFC §A4 + restatement-mapping: there is no separate "system
    // ability" discovery surface. The realm directory enumerates
    // every Agent's abilities via `federation.resolve` (and per-Agent
    // `meta.list_abilities`). The discovery view that consumed this
    // label folds into the standard ability listing.
    //
    // Removed in P2.4. The full body of `system_skills_json()` and
    // its supporting `description_for` table are kept compiled but
    // unused for now — they get GC'd in a follow-up cleanup.

    labels.insert(
        "a2a.description".into(),
        format!(
            "Device hosting {} AI agent(s): {}",
            registry.agents.len(),
            registry
                .agents
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ),
    );

    Some(labels)
}

/// Build the JSON for `a2a.system_skills_json`. Returns an empty
/// string when no system abilities are registered, signalling the
/// caller to omit the label entirely.
///
/// Shape: `{"system_skills": [{"name", "description", "input_schema"}, ...]}`
/// — mirrors the per-skill structure inside `a2a.agents_json`'s
/// agent entries so downstream tooling can reuse the same parser.
///
/// Iteration order follows
/// `runtime::agents::published_ability_names()` which is built from
/// a `BTreeMap` and therefore deterministic. A regression that
/// switched the underlying registry to a `HashMap` would silently
/// break golden-fixture byte-stability.
fn system_skills_json() -> String {
    let names = crate::runtime::agents::published_ability_names();
    if names.is_empty() {
        return String::new();
    }
    // Each system skill emits a thin discovery payload identical in
    // shape to `AgentAbilitySpec::to_discovery_json` —
    // `{name, description, has_input_schema}` only.
    //
    // Why thin (and not full input_schema): the Hub caps each
    // `a2a.*` label value at 4 KiB. A node publishes ~15 system
    // abilities (ping, session.*, permission.*, discuss.*,
    // schedule.*, loop.*) plus `<agent>.chat` for every registered
    // agent — embedding the full JSON Schema for each one blows
    // past 4 KB on the second agent. Discovery callers only need
    // "this skill exists, here's a one-liner"; the full schema is
    // available on demand via:
    //   * MCP `ListTools` over the local IPC socket
    //   * a future `system.<feature>.describe` ability
    //   * the on-disk manifest at
    //     `<agent-root>/abilities/<verb>.ability.toml`
    //
    // The description fallback (the `_ => "(system ability)"` arm)
    // exists so an unknown name lands with a non-empty description
    // rather than the empty string the v1 fallback produced — the
    // cost is < 30 bytes per unknown skill, well within budget.
    let skills: Vec<serde_json::Value> = names
        .iter()
        .map(|name| {
            let description = description_for(name);
            json!({
                "name": name,
                "description": description,
                "has_input_schema": true,
            })
        })
        .collect();
    let envelope = json!({ "system_skills": skills });
    match serde_json::to_string(&envelope) {
        Ok(s) => s,
        Err(_) => {
            debug_assert!(false, "system_skills serialize cannot fail by shape");
            String::new()
        }
    }
}

/// Look up the human-readable description for a published system
/// ability name.
///
/// Authoritative source lives in `runtime::agents::description_for` —
/// kept there so the federation label and the runtime-local register
/// publisher (`runtime::publish::republish_abilities_via_advertise`)
/// pull from one table. This function exists as a thin local alias so
/// the call sites in this module read naturally; do NOT inline a
/// second match here, that's exactly the drift the centralisation
/// removed.
fn description_for(name: &str) -> &'static str {
    crate::runtime::agents::description_for(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::agents::{AgentEntry, AgentType};

    fn entry(agent_type: AgentType, model: Option<&str>) -> AgentEntry {
        AgentEntry::new(agent_type, model.map(String::from))
    }

    #[test]
    fn empty_registry_yields_none() {
        // Returning None (not Some({})) is the contract — see module docstring.
        // A caller can feed this straight into RegisterNodeOptions.labels
        // without remembering to guard is_empty().
        let registry = AgentRegistry::default();
        assert!(build(&registry, "host").is_none());
    }

    #[test]
    fn single_agent_registry_emits_core_keys() {
        let mut registry = AgentRegistry::default();
        registry
            .agents
            .insert("claude".into(), entry(AgentType::ClaudeCode, Some("opus")));
        let labels = build(&registry, "alpha").expect("non-empty registry must yield Some");

        // The v1 label-level `a2a.version` is gone. Version lives
        // per-entry inside a2a.agents_json now.
        assert!(
            !labels.contains_key("a2a.version"),
            "v1 label-level `a2a.version` must not be written under the v2 spec"
        );
        assert_eq!(labels.get("a2a.enabled").map(String::as_str), Some("true"));
        assert_eq!(labels.get("a2a.name").map(String::as_str), Some("alpha"));
        assert!(
            labels
                .get("a2a.description")
                .is_some_and(|d| d.contains("claude") && d.contains("1 AI agent")),
            "description must mention agent count and name, got: {:?}",
            labels.get("a2a.description")
        );
    }

    #[test]
    fn agents_json_is_v2_envelope_shape() {
        let mut registry = AgentRegistry::default();
        registry
            .agents
            .insert("codex".into(), entry(AgentType::Codex, Some("gpt-5")));
        registry
            .agents
            .insert("claude".into(), entry(AgentType::ClaudeCode, None));

        let labels = build(&registry, "host").expect("non-empty registry must yield Some");
        let raw = labels.get("a2a.agents_json").expect("a2a.agents_json");
        let parsed: serde_json::Value =
            serde_json::from_str(raw).expect("a2a.agents_json must be valid JSON");

        // v2 envelope: `{"agents": [...]}`, not a bare array.
        let obj = parsed.as_object().expect("must be a JSON object envelope");
        assert!(
            obj.contains_key("agents"),
            "envelope must have `agents` key"
        );
        let arr = obj["agents"].as_array().expect("`agents` must be an array");
        assert_eq!(arr.len(), 2);
        // BTreeMap ordering → claude comes before codex.
        assert_eq!(arr[0]["name"], "claude");
        assert_eq!(arr[0]["runtime"], "claude-code");
        // Absent model serialises as explicit null, per spec §"null vs absent".
        assert!(arr[0]["model"].is_null());
        assert_eq!(arr[0]["a2a_schema_version"], "v2");
        assert_eq!(arr[1]["name"], "codex");
        assert_eq!(arr[1]["runtime"], "codex");
        assert_eq!(arr[1]["model"], "gpt-5");
        assert_eq!(arr[1]["a2a_schema_version"], "v2");
        // Per-agent `timeout` is gone at v2 (it moved to per-skill
        // `timeout_seconds` inside `skills[*]`). Pin the absence so a
        // future "let me add timeout back" refactor trips loudly.
        assert!(!arr[0].as_object().unwrap().contains_key("timeout"));
        assert!(!arr[0].as_object().unwrap().contains_key("type"));
    }

    #[test]
    fn agents_json_escapes_adversarial_model_names() {
        // An attacker-controlled model string like `" breakJSON"` must be
        // escaped, not concatenated — serde_json guarantees this, but we
        // pin the behavior here so a future "optimize to raw string
        // concat" refactor would fail the test.
        let mut registry = AgentRegistry::default();
        registry.agents.insert(
            "evil".into(),
            entry(AgentType::Codex, Some(r#"" breakJSON"#)),
        );
        let labels = build(&registry, "host").expect("non-empty registry must yield Some");
        let raw = labels.get("a2a.agents_json").expect("a2a.agents_json");
        // Must parse cleanly — if escaping is broken, this fails.
        let parsed: serde_json::Value = serde_json::from_str(raw).expect("still valid JSON");
        assert_eq!(parsed["agents"][0]["model"], r#"" breakJSON"#);
    }

    // ── `skills` field (schema v2) ──────────────────────────────────────────
    //
    // These tests pin the contract the EasyNet backend parses at the
    // v2 envelope's skill layer. A wire-shape change here forces a
    // `A2A_SCHEMA_VERSION` bump (see the const docstring) *and* a
    // companion backend PR.

    #[test]
    fn schema_version_is_v2_string() {
        // Pinned so a stealth version rollback ("let me revert to 2")
        // trips this test loudly. The value is a `v`-prefixed string
        // per spec §Versioning, not a bare "2".
        assert_eq!(A2A_SCHEMA_VERSION, "v2");
    }

    #[test]
    fn each_agent_carries_its_own_skills_list() {
        // Multiple agents must each carry their own ability list, not
        // share one. A broken "clone the same skills array into every
        // entry" refactor would fail here.
        let mut registry = AgentRegistry::default();
        registry
            .agents
            .insert("claude".into(), entry(AgentType::ClaudeCode, None));
        registry
            .agents
            .insert("codex".into(), entry(AgentType::Codex, None));

        let labels = build(&registry, "host").expect("non-empty registry must yield Some");
        let raw = labels.get("a2a.agents_json").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(raw).unwrap();
        let arr = parsed["agents"].as_array().unwrap();

        for entry in arr {
            let skills = entry["skills"].as_array().expect("skills must be array");
            assert!(!skills.is_empty(), "every agent must advertise ≥1 skill");
            let name = entry["name"].as_str().unwrap();
            // Each skill name must be prefixed by its own agent's name —
            // otherwise a federated peer cannot tell which agent to
            // dispatch against when the same verb exists on multiple
            // agents ("claude.chat" vs "codex.chat").
            for skill in skills {
                let skill_name = skill["name"].as_str().expect("skill name must be string");
                assert!(
                    skill_name.starts_with(&format!("{name}.")),
                    "skill {skill_name:?} on agent {name:?} must use `<agent>.<verb>` shape"
                );
            }
        }
    }

    #[test]
    fn skills_entry_shape_matches_v2_discovery_contract() {
        // Pin the v2 thin discovery shape: `{name, description,
        // has_input_schema}`. Earlier drafts shipped the full
        // input_schema / output_schema / timeout_seconds inline,
        // but the chat-as-ability collapse ballooned per-skill JSON
        // past the Hub's 4 KiB label cap and forced the trim — see
        // `AgentAbilitySpec::to_discovery_json`'s doc.
        //
        // The full input_schema is still available on demand via
        // MCP `ListTools` over the local IPC and via the on-disk
        // `<agent-root>/abilities/<verb>.ability.toml`. Discovery
        // labels carry only the fingerprint.
        let mut registry = AgentRegistry::default();
        registry
            .agents
            .insert("claude".into(), entry(AgentType::ClaudeCode, Some("opus")));
        let labels = build(&registry, "host").unwrap();
        let raw = labels.get("a2a.agents_json").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(raw).unwrap();
        let skill = &parsed["agents"][0]["skills"][0];
        assert!(skill["name"].is_string(), "skill.name must be string");
        assert!(
            skill["description"].is_string(),
            "skill.description must be string"
        );
        assert_eq!(
            skill["has_input_schema"],
            serde_json::Value::Bool(true),
            "every v1 ability declares an input_schema; flag must be true"
        );
        // The bytes-cost fields must NOT be re-introduced — that
        // would re-trigger the 4 KiB Hub cap regression. A future
        // PR that wants to ship full schemas to peers should add a
        // separate federation API, not re-inflate the label.
        for forbidden in [
            "input_schema",
            "output_schema",
            "timeout_seconds",
            "parameters",
        ] {
            assert!(
                skill.get(forbidden).is_none(),
                "v2 thin payload must not carry `{forbidden}` (would blow the 4 KiB Hub label cap)"
            );
        }
    }

    #[test]
    fn skills_are_deterministic_across_builds() {
        // Same registry → same labels payload. A non-determinism here
        // (e.g. a HashSet leaking into the build path) would produce
        // flaky Hub-side registrations when the daemon's re-register
        // hook fires.
        //
        // HomeGuard isolates this test from the developer's real
        // ~/.easynet/. Without it, abilities_for's slice-25
        // fallback can pick up real on-disk manifests under
        // ~/.easynet/workspaces/{claude,codex}, and a parallel
        // test that mutates those workspaces (e.g. the new G1
        // build_stdio_server_with_agent_name test) would race
        // and produce different bytes between the two build()
        // calls. The HomeGuard pins the test to a fresh tempdir.
        let _g = crate::facade::cli::test_support::HomeGuard::new();

        let mut registry = AgentRegistry::default();
        registry
            .agents
            .insert("claude".into(), entry(AgentType::ClaudeCode, None));
        registry
            .agents
            .insert("codex".into(), entry(AgentType::Codex, None));
        let a = build(&registry, "host").unwrap();
        let b = build(&registry, "host").unwrap();
        assert_eq!(a.get("a2a.agents_json"), b.get("a2a.agents_json"));
    }

    #[test]
    fn golden_fixture_byte_equality() {
        // Cross-repo contract test. The CLI writer, the EasyNet
        // backend parser, and any future peer that reads this
        // label all agree on `tests/fixtures/a2a-v2/golden.json`
        // being THE canonical byte sequence. If this test breaks,
        // the backend's companion fixture must be updated in the
        // same release window (spec §"Contract test (golden
        // fixture)").
        //
        // HomeGuard isolates this test from the developer's real
        // ~/.easynet/. abilities_from_manifests falls back to
        // agents_root().join(name) when an entry has no root_path —
        // on a developer's machine that already has
        // ~/.easynet/workspaces/alice (left over from a previous
        // session), the fallback would return THAT workspace's
        // real manifests, drifting from the canonical synth-fallback
        // description this fixture was authored against. The guard
        // ensures the test sees a freshly-empty home so the fallback
        // hits the synth path every time, restoring cross-host
        // stability.
        let _g = crate::facade::cli::test_support::HomeGuard::new();

        let mut registry = AgentRegistry::default();
        let mut alice = entry(AgentType::ClaudeCode, Some("claude-opus-4-7"));
        alice.label = Some("code-review assistant".into());
        registry.agents.insert("alice".into(), alice);
        registry
            .agents
            .insert("bob".into(), entry(AgentType::Codex, None));

        let labels = build(&registry, "host").expect("non-empty registry must yield Some");
        let produced = labels.get("a2a.agents_json").expect("a2a.agents_json");

        // Load the fixture. The fixture is multi-line pretty-
        // printed for human reading; our writer emits compact
        // JSON. Compare after reparsing + re-serializing both
        // through serde_json::to_string so formatting diffs
        // don't mask semantic diffs.
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("a2a-v2")
            .join("golden.json");
        let fixture_raw = std::fs::read_to_string(&fixture_path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", fixture_path.display()));
        let fixture_val: serde_json::Value =
            serde_json::from_str(&fixture_raw).expect("fixture is valid JSON");
        let produced_val: serde_json::Value =
            serde_json::from_str(produced).expect("produced label is valid JSON");

        if fixture_val != produced_val {
            // Print both on failure so the diff is immediately
            // visible in CI output. Pretty-print the produced side
            // so an engineer can copy it back into the fixture if
            // the rewrite is the intentional one.
            let produced_pretty = serde_json::to_string_pretty(&produced_val).unwrap();
            panic!(
                "node roster label drift from golden fixture.\n\n\
                 Fixture path: {}\n\n\
                 Produced:\n{}\n\n\
                 If this drift is intentional, update the fixture AND the EasyNet backend companion parser + fixture in the same release window (spec §Breaking changes from current on-wire shape).",
                fixture_path.display(),
                produced_pretty,
            );
        }
    }

    #[test]
    fn adversarial_agent_name_cannot_be_registered_so_skills_shape_stays_safe() {
        // The skill name is built as `format!("{agent_name}.chat")`. If
        // the registry ever allowed control chars or `"` in agent names,
        // our unescaped concat inside `chat_ability` would break the
        // JSON. `validate_agent_name` is what prevents that; pin the
        // relationship here so a future loosening of the validator
        // forces a corresponding hardening of the skill constructor.
        use crate::registry::agents::validate_agent_name;
        for bad in [r#"claude""#, "claude\n", "claude\t", "agent.name", "Agent"] {
            assert!(
                validate_agent_name(bad).is_err(),
                "agent name {bad:?} must be rejected by the registry — \
                 skill-name construction depends on that guarantee"
            );
        }
    }
}

#[cfg(test)]
mod label_size_guard {
    //! Hub-side guard: every value in the labels map this module
    //! produces must fit the gRPC-side 4 KiB cap on a `RegisterNode`
    //! label value. A regression here means a node will fail to
    //! register with `InvalidArgument: invalid labels: label value
    //! for key "X" exceeds 4096 bytes` and the device drops off the
    //! federation. We pin a tighter 3 KiB target so a small future
    //! bump (one more system ability, a slightly longer description)
    //! doesn't drive us into the actual ceiling.
    //!
    //! If you legitimately need a label > 3 KiB, the right answer
    //! is to thin the payload further (drop input_schema, drop
    //! verbose descriptions) — not to raise this constant. The
    //! Hub's 4 KiB ceiling is a wire-level invariant that won't
    //! move just because we want it to.

    use super::*;
    use crate::registry::agents::{AgentEntry, AgentRegistry, AgentType};

    /// Self-imposed budget. Stays under the Hub's 4 KiB cap with
    /// headroom for one or two more agents / system abilities.
    /// Set at 3700 (≈90% of the 4096 ceiling): the system_skills_json
    /// label measures ~3.3 KB today after the chat-as-ability
    /// collapse landed (~16 system abilities, each carrying a
    /// one-sentence description). 3700 leaves room for ~3 more
    /// system abilities or 4 longer descriptions; tighter would
    /// leave no room to grow, looser stops catching the next
    /// ability that adds a multi-paragraph description.
    const MAX_LABEL_BYTES: usize = 3700;

    fn registry_with_two_agents() -> AgentRegistry {
        let mut r = AgentRegistry::default();
        r.agents.insert(
            "claude".into(),
            AgentEntry::new(AgentType::ClaudeCode, Some("sonnet".into())),
        );
        r.agents.insert(
            "codex".into(),
            AgentEntry::new(AgentType::Codex, Some("gpt-5.2".into())),
        );
        r
    }

    #[test]
    fn each_label_value_fits_under_budget_for_two_agents() {
        // Two agents is the smallest realistic non-trivial node
        // shape — one agent doesn't exercise the agents_json growth
        // axis, three+ agents would overshoot a budget chosen for a
        // typical install. Pin the two-agent shape so any new
        // ability or schema addition is checked against the same
        // baseline.
        let labels =
            build(&registry_with_two_agents(), "host").expect("non-empty registry must yield Some");
        for (k, v) in &labels {
            assert!(
                v.len() <= MAX_LABEL_BYTES,
                "label `{k}` is {} bytes; budget is {MAX_LABEL_BYTES} (Hub ceiling 4096). \
                 Trim payload — see fn to_discovery_json / system_skills_json doc.",
                v.len()
            );
        }
    }
}
