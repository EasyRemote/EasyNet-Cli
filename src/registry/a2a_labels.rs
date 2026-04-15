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

/// Schema version emitted as `a2a.version`. Bump only on a wire change
/// that older consumers cannot interpret; additive changes (new optional
/// keys inside `a2a.agents_json`) do NOT warrant a bump.
pub const A2A_LABEL_SCHEMA_VERSION: &str = "1";

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
    // Stamp the schema version first so it is always present on the
    // wire, even if the rest of the map is empty for whatever reason.
    // The Hub's tolerant-read path uses this to branch on layout
    // without pattern-matching against absence.
    labels.insert("a2a.version".into(), A2A_LABEL_SCHEMA_VERSION.to_string());
    labels.insert("a2a.enabled".into(), "true".into());
    labels.insert("a2a.name".into(), hostname.to_string());

    // Encode full agent details as JSON so the backend can reconstruct
    // individual agent entries from a single label value. We build this
    // via serde_json so any `"` / `\` in model names or custom labels is
    // escaped correctly.
    //
    // `serde_json::to_string` can only fail when the Value contains
    // NaN/Infinity numbers or non-string map keys; our shape here is an
    // array of objects whose values are plain strings and a u64, so the
    // serializer has no observable way to return `Err`. Even so, we do
    // NOT `expect` — a release-build label registration is not worth
    // aborting the whole runtime over a theoretical serializer change.
    // We `debug_assert!` in development (to surface any future regression
    // loud and fast) and fall back to an empty JSON array in release
    // (the node stays registered without the agent roster; consumers
    // reading `a2a.agents_json` already tolerate missing/empty arrays).
    let agents_json: Vec<serde_json::Value> = registry
        .agents
        .iter()
        .map(|(name, e)| {
            json!({
                "name": name,
                "type": e.agent_type.to_string(),
                "model": e.model.as_deref().unwrap_or(""),
                "timeout": e.timeout_secs,
            })
        })
        .collect();
    let agents_json_str = match serde_json::to_string(&agents_json) {
        Ok(s) => s,
        Err(_e) => {
            debug_assert!(
                false,
                "a2a.agents_json serialization failed — our shape cannot produce NaN/Infinity or non-string keys; a serde_json behavior change must have broken the invariant",
            );
            "[]".to_string()
        }
    };
    labels.insert("a2a.agents_json".into(), agents_json_str);

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

        assert_eq!(
            labels.get("a2a.version").map(String::as_str),
            Some(A2A_LABEL_SCHEMA_VERSION),
            "version label must be stamped on every non-empty registration"
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
    fn agents_json_is_parseable_and_well_shaped() {
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

        let arr = parsed.as_array().expect("must be an array");
        assert_eq!(arr.len(), 2);
        // BTreeMap ordering → claude comes before codex.
        assert_eq!(arr[0]["name"], "claude");
        assert_eq!(arr[0]["type"], "claude-code");
        assert_eq!(arr[0]["model"], "");
        assert_eq!(arr[1]["name"], "codex");
        assert_eq!(arr[1]["type"], "codex");
        assert_eq!(arr[1]["model"], "gpt-5");
        assert!(arr[0]["timeout"].is_u64(), "timeout must be numeric");
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
        assert_eq!(parsed[0]["model"], r#"" breakJSON"#);
    }
}
