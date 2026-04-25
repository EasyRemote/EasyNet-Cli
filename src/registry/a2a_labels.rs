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
            // Optional fields carry explicit `null` rather than
            // being omitted. Spec §"null vs absent" fixes the
            // writer rule; fixture byte-stability depends on it.
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
    let envelope = json!({ "agents": agents_json });
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

    // PR-SYS: device-level system abilities published as a separate
    // label key so v2-only parsers (which read `a2a.agents_json` and
    // ignore unknown labels) keep working unchanged. v3-aware
    // parsers will look for `a2a.system_skills_json` and merge into
    // their discovery view.
    //
    // Why a separate label rather than a new envelope field on
    // a2a.agents_json:
    //   * tests/fixtures/a2a-v2/golden.json byte-stability is a
    //     CI invariant; introducing a new field at the envelope
    //     level would force a coordinated backend release.
    //   * The 32 KiB per-label limit is enforced per key. With
    //     two separate labels each gets its own budget.
    //   * Disambiguates "agent abilities" from "device abilities"
    //     in label-grep tooling.
    let system_skills_json = system_skills_json();
    if !system_skills_json.is_empty() {
        labels.insert("a2a.system_skills_json".into(), system_skills_json);
    }

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
/// `runtime::system::published_ability_names()` which is built from
/// a `BTreeMap` and therefore deterministic. A regression that
/// switched the underlying registry to a `HashMap` would silently
/// break golden-fixture byte-stability.
fn system_skills_json() -> String {
    let names = crate::runtime::system::published_ability_names();
    if names.is_empty() {
        return String::new();
    }
    // Map each name to the per-skill JSON. v1 hardcodes the
    // mapping inline because the only ability is `system.ping`;
    // PR-ATTACH onwards adds a per-ability `to_discovery_json()`
    // helper next to each handler so this match becomes obsolete.
    let skills: Vec<serde_json::Value> = names
        .iter()
        .map(|name| match name.as_str() {
            "system.ping" => json!({
                "name": "system.ping",
                "description": crate::runtime::system::ping::description(),
                "input_schema": crate::runtime::system::ping::input_schema(),
                "output_schema": serde_json::Value::Null,
                "timeout_seconds": serde_json::Value::Null,
            }),
            "system.session.list" => json!({
                "name": "system.session.list",
                "description": crate::runtime::system::session_ability::list_description(),
                "input_schema":
                    crate::runtime::system::session_ability::list_input_schema(),
                "output_schema": serde_json::Value::Null,
                "timeout_seconds": serde_json::Value::Null,
            }),
            "system.session.attach" => json!({
                "name": "system.session.attach",
                "description":
                    crate::runtime::system::session_ability::attach_description(),
                "input_schema":
                    crate::runtime::system::session_ability::attach_input_schema(),
                "output_schema": serde_json::Value::Null,
                "timeout_seconds": serde_json::Value::Null,
            }),
            "system.permission.subscribe" => json!({
                "name": "system.permission.subscribe",
                "description":
                    crate::runtime::system::permission_ability::subscribe_description(),
                "input_schema":
                    crate::runtime::system::permission_ability::subscribe_input_schema(),
                "output_schema": serde_json::Value::Null,
                "timeout_seconds": serde_json::Value::Null,
            }),
            "system.permission.decide" => json!({
                "name": "system.permission.decide",
                "description":
                    crate::runtime::system::permission_ability::decide_description(),
                "input_schema":
                    crate::runtime::system::permission_ability::decide_input_schema(),
                "output_schema": serde_json::Value::Null,
                "timeout_seconds": serde_json::Value::Null,
            }),
            "system.discuss.create" => json!({
                "name": "system.discuss.create",
                "description":
                    crate::runtime::system::discuss_ability::create_description(),
                "input_schema":
                    crate::runtime::system::discuss_ability::create_input_schema(),
                "output_schema": serde_json::Value::Null,
                "timeout_seconds": serde_json::Value::Null,
            }),
            "system.discuss.post" => json!({
                "name": "system.discuss.post",
                "description":
                    crate::runtime::system::discuss_ability::post_description(),
                "input_schema":
                    crate::runtime::system::discuss_ability::post_input_schema(),
                "output_schema": serde_json::Value::Null,
                "timeout_seconds": serde_json::Value::Null,
            }),
            "system.discuss.subscribe" => json!({
                "name": "system.discuss.subscribe",
                "description":
                    crate::runtime::system::discuss_ability::subscribe_description(),
                "input_schema":
                    crate::runtime::system::discuss_ability::subscribe_input_schema(),
                "output_schema": serde_json::Value::Null,
                "timeout_seconds": serde_json::Value::Null,
            }),
            other => json!({
                "name": other,
                "description": "",
                "input_schema": {"type": "object", "additionalProperties": true},
                "output_schema": serde_json::Value::Null,
                "timeout_seconds": serde_json::Value::Null,
            }),
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
        assert!(obj.contains_key("agents"), "envelope must have `agents` key");
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
        // Pin the exact keys on every skill object per spec
        // §"Agent entry" — `name`, `description`, `input_schema`,
        // `output_schema`, `timeout_seconds`. A rename here is a
        // wire break that the backend companion parser would catch
        // first, but pinning here surfaces the break at the CLI's
        // own test run rather than only at cross-repo CI.
        let mut registry = AgentRegistry::default();
        registry
            .agents
            .insert("claude".into(), entry(AgentType::ClaudeCode, Some("opus")));
        let labels = build(&registry, "host").unwrap();
        let raw = labels.get("a2a.agents_json").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(raw).unwrap();
        let skill = &parsed["agents"][0]["skills"][0];
        assert!(skill["name"].is_string(), "skill.name must be string");
        assert!(skill["description"].is_string(), "skill.description must be string");
        assert!(
            skill["input_schema"].is_object(),
            "skill.input_schema must be a JSON schema object"
        );
        assert_eq!(skill["input_schema"]["type"], "object");
        // output_schema and timeout_seconds are present as explicit
        // `null` on the seeded chat ability, per the writer rule in
        // spec §"null vs absent".
        assert!(skill["output_schema"].is_null(), "output_schema is null on chat");
        assert!(skill["timeout_seconds"].is_null(), "timeout_seconds is null on chat");
        // v1 `parameters` key must be gone.
        assert!(
            skill.get("parameters").is_none(),
            "v1 `parameters` key must not appear in v2 output"
        );
    }

    #[test]
    fn skills_are_deterministic_across_builds() {
        // Same registry → same labels payload. A non-determinism here
        // (e.g. a HashSet leaking into the build path) would produce
        // flaky Hub-side registrations when the daemon's re-register
        // hook fires.
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
        let mut registry = AgentRegistry::default();
        let mut alice = entry(AgentType::ClaudeCode, Some("claude-opus-4-7"));
        alice.label = Some("code-review assistant".into());
        registry.agents.insert("alice".into(), alice);
        registry.agents.insert("bob".into(), entry(AgentType::Codex, None));

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
        let fixture_val: serde_json::Value = serde_json::from_str(&fixture_raw)
            .expect("fixture is valid JSON");
        let produced_val: serde_json::Value = serde_json::from_str(produced)
            .expect("produced label is valid JSON");

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
        for bad in [
            r#"claude""#,
            "claude\n",
            "claude\t",
            "agent.name",
            "Agent",
        ] {
            assert!(
                validate_agent_name(bad).is_err(),
                "agent name {bad:?} must be rejected by the registry — \
                 skill-name construction depends on that guarantee"
            );
        }
    }
}
