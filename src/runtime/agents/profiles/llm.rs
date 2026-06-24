//! llm profile — RFC-001 §1.
//!
//! An Agent advertising conversation.* plus per-skill abilities. One per
//! registered AI sub-agent (claude / codex / future). Per RFC §1.1 + §A15:
//! each LLM sub-agent's installed skills are PRIVATE abilities advertised by
//! that sub-agent projection.
//!
//! Descriptor projection
//! ---------------------
//! LLM profile descriptors are dynamic because each registered AI sub-agent has
//! its own `<agent>.chat` and manifest-provided abilities. Static system-registry
//! projection is therefore not enough for this profile. This module keeps the
//! dynamic LLM ability-shape filter private; no other module may use it as a
//! generic projection classifier.
//!
//! Currently wired in agents/chat_ability.rs (which renames to
//! conversation_ability.rs in a follow-up cleanup). The chat handler
//! is the conversation.send implementation.

const LLM_DYNAMIC_ABILITY_PREFIXES: &[&str] = &[
    // RFC-005 owner-local catalogue names: `meta.*` and the built-in
    // `session.*` and `skill.<operation>` abilities are device-profile-owned.
    // LLM profile's dynamic surface is `conversation.*` plus private
    // per-skill `skill.<skill-name>` entries.
    //
    // `voice.*` was previously listed here on the
    // assumption that voice signaling is an LLM-owned ability
    // family (RFC-005 v3.2 A6/A7). The handlers actually run
    // on the device daemon (`OwnerKind::Device` in the
    // registry); claiming them in this profile caused the
    // catalogue's `descriptors_for(agent_ura)` to stamp every
    // voice verb with the agent URA, so `easynet ability list`
    // grouped them under AGENT instead of DEVICE / SYSTEM and
    // the KIND column read `agent`. Per the truth-table spec
    // ownership reflects "where does the handler run", not
    // "which surface category it semantically belongs to" —
    // voice is host-owned (microphone / camera / speaker
    // hardware lives on the device) and is now described through
    // registry `OwnerKind::Device`, not through an LLM prefix claim.
    "conversation.",
    "skill.",
];

fn is_llm_dynamic_ability(ability_name: &str) -> bool {
    const DEVICE_SKILL_ABILITIES: &[&str] = &[
        "skill.install",
        "skill.list",
        "skill.publish",
        "skill.read_file",
        "skill.remove",
        "skill.tree",
        "skill.unpublish",
        "skill.upgrade",
        "skill.write_file",
    ];
    if DEVICE_SKILL_ABILITIES.contains(&ability_name) {
        return false;
    }
    LLM_DYNAMIC_ABILITY_PREFIXES
        .iter()
        .any(|p| ability_name.starts_with(p))
}

/// AbilityDescriptors for every conversation.* / private skill.* in the live
/// registry, anchored to the LLM-profile Agent's URA. Per RFC §1.1 + §18:
///   * skill.*         → PRIVATE (per-skill, owner-only by default)
///   * conversation.*  → SCOPED  (default per [P8] correction)
///
/// P4.7 narrows the SCOPED axes.
pub fn descriptors_for(
    owner_ura: &str,
) -> Vec<crate::runtime::ability_descriptor::AbilityDescriptor> {
    descriptors_for_with_metadata(owner_ura, None)
}

/// Same as `descriptors_for`, but stamps each emitted descriptor's
/// `metadata["agent_type"]` with the given string when supplied.
///
/// Per RFC §A4 the wire-level Agent envelope has no `kind` /
/// `type` field. The legacy `registry::AgentType` Rust enum
/// (claude-code | codex | codex-app-server) is intentionally kept
/// as an internal type — refactoring 28+ files to delete it is
/// out of scope here — but its display string is surfaced through
/// the descriptor's open-ended `metadata` bag so downstream
/// consumers (Frontend Agents page, `meta.describe`) can render
/// it without a protocol-level discriminator.
///
/// Callers that don't know the agent type pass `None`; only the
/// agent-registry dispatcher (which loads `registry::agents`) has
/// the type in hand at advertise time.
pub fn descriptors_for_with_metadata(
    owner_ura: &str,
    agent_type_display: Option<&str>,
) -> Vec<crate::runtime::ability_descriptor::AbilityDescriptor> {
    use crate::runtime::ability_descriptor::{AbilityDescriptor, Visibility};
    crate::runtime::agents::published_abilities()
        .into_iter()
        .filter(|m| is_llm_dynamic_ability(&m.name))
        .map(|m| {
            let visibility = if m.name.starts_with("skill.") {
                Visibility::Private
            } else {
                Visibility::Scoped
            };
            let mut desc = AbilityDescriptor::new(m.name.clone(), owner_ura, visibility)
                .expect("registry-derived names satisfy descriptor invariants")
                .with_input_schema(m.input_schema.clone())
                .with_hints(m.hints.clone())
                .with_source("kernel:built-in")
                .with_description(m.description);
            if let Some(t) = agent_type_display {
                desc = desc.with_metadata_entry("agent_type", t);
            }
            desc
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_filter_recognizes_llm_namespaces() {
        // Q3 of truth-table spec: meta.* and skill.* are
        // device-profile-owned, NOT llm-profile. The LLM surface is
        // conversation.* and private skill.*.
        assert!(is_llm_dynamic_ability("conversation.send"));
        assert!(is_llm_dynamic_ability("conversation.stream"));
        assert!(!is_llm_dynamic_ability("session.list"));
        assert!(!is_llm_dynamic_ability("session.attach"));
        assert!(is_llm_dynamic_ability("skill.alive-video"));
        assert!(is_llm_dynamic_ability("skill.design"));
        assert!(!is_llm_dynamic_ability("skill.list"));
        assert!(!is_llm_dynamic_ability("skill.tree"));
        // voice.* is NOT llm-owned post-truth-table fix —
        // the handlers run on the device daemon.
        assert!(!is_llm_dynamic_ability("voice.subscribe"));
        // meta.* is NOT llm-owned post-M2.
        assert!(!is_llm_dynamic_ability("meta.describe"));
    }

    #[test]
    fn dynamic_filter_rejects_other_profiles() {
        assert!(!is_llm_dynamic_ability("skill.list"));
        assert!(!is_llm_dynamic_ability("consent.subscribe"));
    }

    #[test]
    fn descriptors_for_marks_skill_namespace_as_private() {
        use crate::runtime::ability_descriptor::Visibility;
        let descriptors = descriptors_for("easynet:///r/acme/agent/u1.01LLM");
        for d in descriptors {
            if d.name.starts_with("skill.") {
                assert_eq!(
                    d.visibility,
                    Visibility::Private,
                    "{} must be PRIVATE",
                    d.name
                );
            } else {
                assert_eq!(
                    d.visibility,
                    Visibility::Scoped,
                    "{} must be SCOPED",
                    d.name
                );
            }
        }
    }

    #[test]
    fn descriptors_for_with_metadata_stamps_agent_type_when_provided() {
        let descriptors =
            descriptors_for_with_metadata("easynet:///r/acme/agent/u1.01LLM", Some("claude-code"));
        for d in &descriptors {
            assert_eq!(
                d.metadata.get("agent_type").map(String::as_str),
                Some("claude-code"),
                "{} must carry agent_type metadata when caller knows it",
                d.name,
            );
        }
    }

    #[test]
    fn descriptors_for_with_metadata_omits_agent_type_when_absent() {
        let descriptors = descriptors_for_with_metadata("easynet:///r/acme/agent/u1.01LLM", None);
        for d in &descriptors {
            assert!(
                !d.metadata.contains_key("agent_type"),
                "{} must NOT fabricate agent_type when caller didn't supply one",
                d.name,
            );
        }
    }
}
