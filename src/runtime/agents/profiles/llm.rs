//! llm profile — RFC-001 §1.
//!
//! An Agent advertising conversation.* + session.* + meta.* +
//! per-skill abilities. One per registered AI sub-agent (claude /
//! codex / future). Per RFC §1.1 + §A15: each LLM sub-agent's
//! installed skills are PRIVATE abilities owned by that sub-agent.
//!
//! Owned ability namespaces
//! ------------------------
//!   conversation.send / conversation.stream  (default visibility SCOPED [P8])
//!   session.create / session.list / session.resume / session.close
//!   meta.describe / meta.list_abilities / meta.acquire / meta.forget
//!     / meta.publish / meta.compose / meta.cancel
//!   skill.<name>  (per-skill PRIVATE abilities; one per directory in
//!                  ~/.claude/skills/, ~/.agents/skills/, etc.)
//!
//! Currently wired in agents/chat_ability.rs (which renames to
//! conversation_ability.rs in a follow-up cleanup). The chat handler
//! is the conversation.send implementation.

pub const LLM_PROFILE_ABILITY_PREFIXES: &[&str] = &[
    "conversation.",
    "session.",
    "meta.",
    "skill.",
];

pub fn owns(ability_name: &str) -> bool {
    LLM_PROFILE_ABILITY_PREFIXES
        .iter()
        .any(|p| ability_name.starts_with(p))
}

/// AbilityDescriptors for every conversation.* / session.* / meta.*
/// / skill.* in the live registry, anchored to the LLM-profile
/// Agent's URA. Per RFC §1.1 + §18:
///   * skill.*         → PRIVATE (per-skill, owner-only by default)
///   * conversation.*  → SCOPED  (default per [P8] correction)
///   * session.*       → SCOPED
///   * meta.*          → SCOPED  (callable PUBLIC, results filtered)
///
/// `meta.describe` is technically PUBLIC per §18, but we mark it
/// SCOPED here for safety; the higher-level dispatcher upgrades to
/// PUBLIC when it lands. P4.7 narrows the SCOPED axes.
pub fn descriptors_for(
    owner_agent_uri: &str,
) -> Vec<crate::runtime::ability_descriptor::AbilityDescriptor> {
    descriptors_for_with_metadata(owner_agent_uri, None)
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
/// fleet-level dispatcher (which loads `registry::agents`) has
/// the type in hand at advertise time.
pub fn descriptors_for_with_metadata(
    owner_agent_uri: &str,
    agent_type_display: Option<&str>,
) -> Vec<crate::runtime::ability_descriptor::AbilityDescriptor> {
    use crate::runtime::ability_descriptor::{AbilityDescriptor, Visibility};
    crate::runtime::agents::published_abilities()
        .into_iter()
        .filter(|m| owns(&m.name))
        .map(|m| {
            let visibility = if m.name.starts_with("skill.") {
                Visibility::Private
            } else {
                Visibility::Scoped
            };
            let mut desc =
                AbilityDescriptor::new(m.name.clone(), owner_agent_uri, visibility)
                    .expect("registry-derived names satisfy descriptor invariants")
                    .with_input_schema(m.input_schema.clone())
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
    fn owns_recognizes_llm_namespaces() {
        assert!(owns("conversation.send"));
        assert!(owns("conversation.stream"));
        assert!(owns("session.create"));
        assert!(owns("session.resume"));
        assert!(owns("meta.describe"));
        assert!(owns("meta.list_abilities"));
        assert!(owns("skill.alive-video"));
        assert!(owns("skill.design"));
    }

    #[test]
    fn owns_rejects_other_profiles() {
        assert!(!owns("fleet.list_abilities"));
        assert!(!owns("consent.subscribe"));
        assert!(!owns("policy.evaluate"));
    }

    #[test]
    fn descriptors_for_marks_skill_namespace_as_private() {
        use crate::runtime::ability_descriptor::Visibility;
        let descriptors = descriptors_for("easynet:///r/acme/agent/01LLM");
        for d in descriptors {
            if d.name.starts_with("skill.") {
                assert_eq!(d.visibility, Visibility::Private, "{} must be PRIVATE", d.name);
            } else {
                assert_eq!(d.visibility, Visibility::Scoped, "{} must be SCOPED", d.name);
            }
        }
    }

    #[test]
    fn descriptors_for_with_metadata_stamps_agent_type_when_provided() {
        let descriptors = descriptors_for_with_metadata(
            "easynet:///r/acme/agent/01LLM",
            Some("claude-code"),
        );
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
        let descriptors = descriptors_for_with_metadata(
            "easynet:///r/acme/agent/01LLM",
            None,
        );
        for d in &descriptors {
            assert!(
                !d.metadata.contains_key("agent_type"),
                "{} must NOT fabricate agent_type when caller didn't supply one",
                d.name,
            );
        }
    }
}
