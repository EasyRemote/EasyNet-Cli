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
//!   device.meta.describe / device.meta.list_abilities / device.meta.acquire / device.meta.forget
//!     / device.meta.publish / device.meta.compose / device.meta.cancel
//!   skill.<name>  (per-skill PRIVATE abilities; one per directory in
//!                  ~/.claude/skills/, ~/.agents/skills/, etc.)
//!
//! Currently wired in agents/chat_ability.rs (which renames to
//! conversation_ability.rs in a follow-up cleanup). The chat handler
//! is the conversation.send implementation.

pub const LLM_PROFILE_ABILITY_PREFIXES: &[&str] = &[
    // M2/M3 of system-namespace migration: catalogue uses
    // canonical `device.*` partition. Per Q3 of the truth-table
    // spec, `meta.*` is device-profile-owned (device-introspection
    // is a host concern; per-agent introspection uses the
    // `{ scope: "<self>" }` parameter). LLM profile's surface is
    // `conversation.*` / `session.*` / `<agent>.skill.*`. Legacy
    // bare-namespace prefixes are kept as a defense-in-depth
    // fallback for stale call sites that emit a legacy-named
    // ability into a profile-side selector.
    //
    // `device.voice.*` was previously listed here on the
    // assumption that voice signaling is an LLM-owned ability
    // family (RFC-005 v3.2 A6/A7). The handlers actually run
    // on the device daemon (`OwnerKind::Device` in the
    // registry); claiming them in this profile caused the
    // catalogue's `descriptors_for(agent_uri)` to stamp every
    // voice verb with the agent URA, so `easynet ability list`
    // grouped them under AGENT instead of DEVICE / SYSTEM and
    // the KIND column read `agent`. Per the truth-table spec
    // ownership reflects "where does the handler run", not
    // "which surface category it semantically belongs to" —
    // voice is host-owned (microphone / camera / speaker
    // hardware lives on the device). Moved to the device
    // profile via DEVICE_PROFILE_ABILITY_PREFIXES.
    "conversation.",
    "session.",
    "device.skill.",
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
/// `device.meta.describe` is technically PUBLIC per §18, but we mark it
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
/// consumers (Frontend Agents page, `device.meta.describe`) can render
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
            let mut desc = AbilityDescriptor::new(m.name.clone(), owner_agent_uri, visibility)
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
    fn owns_recognizes_llm_namespaces() {
        // Q3 of truth-table spec: meta.* is device-profile-owned,
        // NOT llm-profile. The LLM surface is conversation.*,
        // session.*, device.skill.*. meta.* and device.voice.*
        // fall through to the device profile (the voice signaling
        // handlers run on the host daemon).
        assert!(owns("conversation.send"));
        assert!(owns("conversation.stream"));
        assert!(owns("session.create"));
        assert!(owns("session.resume"));
        assert!(owns("device.skill.alive-video"));
        assert!(owns("device.skill.design"));
        // device.voice.* is NOT llm-owned post-truth-table fix —
        // the handlers run on the device daemon.
        assert!(!owns("device.voice.subscribe"));
        // meta.* is NOT llm-owned post-M2.
        assert!(!owns("device.meta.describe"));
    }

    #[test]
    fn owns_rejects_other_profiles() {
        assert!(!owns("device.fleet.list_abilities"));
        assert!(!owns("device.consent.subscribe"));
        assert!(!owns("device.policy.evaluate"));
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
