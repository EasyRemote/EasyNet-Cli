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
//! Currently wired in system_abilities/agents/chat.rs. The chat handler
//! is the conversation.send implementation.

use crate::daemon::ability::descriptors::AbilityDescriptor;

const LLM_DYNAMIC_ABILITY_PREFIXES: &[&str] = &[
    // RFC-005 owner-local catalogue names: `meta.*` and the built-in
    // `session.*` and `skill.<operation>` abilities are device-profile-owned.
    // LLM profile's dynamic surface is `conversation.*` plus private
    // per-skill `skill.<skill-name>` entries.
    //
    // `voice.*` was previously listed here on the
    // assumption that voice signaling is an LLM-owned ability
    // family (RFC-005 v3.2 A6/A7). The handlers actually run
    // under the realm Authority (`OwnerKind::RealmAuthority` in the
    // registry); claiming them in this profile caused the
    // catalogue's `descriptors_for(agent_ura)` to stamp every
    // voice verb with the agent URA, so `easynet ability list`
    // grouped them under AGENT instead of DEVICE / SYSTEM and
    // the KIND column read `agent`. Per the truth-table spec
    // ownership reflects "where does the handler run", not
    // "which surface category it semantically belongs to" —
    // microphone / camera / speaker hardware remains Device-owned,
    // while voice synthesis, transcription, and signaling are now
    // described through registry `OwnerKind::RealmAuthority`, not through an
    // LLM prefix claim.
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
) -> Vec<crate::daemon::ability::descriptors::AbilityDescriptor> {
    let catalog = LlmProfileAbilityCatalog::load();
    descriptors_for_with_catalog(owner_ura, None, &catalog)
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
) -> Vec<crate::daemon::ability::descriptors::AbilityDescriptor> {
    let catalog = LlmProfileAbilityCatalog::load();
    descriptors_for_with_catalog(owner_ura, agent_type_display, &catalog)
}

/// Pre-filtered system ability catalogue for LLM-profile projection.
///
/// `published_abilities()` builds the live system catalogue and its transport
/// hint snapshot. Calling it once per hosted LLM turns `meta.list_abilities`
/// into O(hosted_agents * system_abilities). This value object makes the read
/// model explicit: one catalogue snapshot, then a cheap owner-specific
/// projection for each hosted agent.
#[derive(Debug, Clone)]
pub struct LlmProfileAbilityCatalog {
    abilities: Vec<AbilityDescriptor>,
}

impl LlmProfileAbilityCatalog {
    #[must_use]
    pub fn load() -> Self {
        Self::from_system_abilities(crate::daemon::ability::catalog::published_abilities())
    }

    #[must_use]
    pub fn from_system_abilities(abilities: Vec<AbilityDescriptor>) -> Self {
        Self {
            abilities: abilities
                .into_iter()
                .filter(|m| is_llm_dynamic_ability(&m.name))
                .collect(),
        }
    }

    fn iter(&self) -> impl Iterator<Item = &AbilityDescriptor> {
        self.abilities.iter()
    }
}

pub fn descriptors_for_with_catalog(
    owner_ura: &str,
    agent_type_display: Option<&str>,
    catalog: &LlmProfileAbilityCatalog,
) -> Vec<crate::daemon::ability::descriptors::AbilityDescriptor> {
    use crate::daemon::ability::descriptors::Visibility;
    catalog
        .iter()
        .map(|descriptor| {
            let visibility = if descriptor.name.starts_with("skill.") {
                Visibility::Private
            } else {
                Visibility::Scoped
            };
            let mut desc = descriptor
                .clone()
                .rebind_owner_ura(owner_ura)
                .expect("registry-derived descriptor accepts canonical LLM owner")
                .with_visibility(visibility)
                .with_source("kernel:built-in");
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
        assert!(!is_llm_dynamic_ability(
            crate::daemon::ability::names::device_control::SESSION_LIST
        ));
        assert!(!is_llm_dynamic_ability("session.attach"));
        assert!(is_llm_dynamic_ability("skill.alive-video"));
        assert!(is_llm_dynamic_ability("skill.design"));
        assert!(!is_llm_dynamic_ability("skill.list"));
        assert!(!is_llm_dynamic_ability("skill.tree"));
        // voice.* is realm Authority-owned post-truth-table cutover.
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
        use crate::daemon::ability::descriptors::Visibility;
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

    #[test]
    fn catalog_snapshot_projects_to_multiple_llm_owners_without_mutating_source() {
        let source_owner = "easynet:///r/acme/device/catalog";
        let descriptor = |name: &str, description: &str| {
            AbilityDescriptor::new(
                name,
                source_owner,
                crate::daemon::ability::descriptors::Visibility::Scoped,
                crate::daemon::ability::descriptors::AdmissionAction::Invoke,
            )
            .expect("test descriptor")
            .with_description(description)
            .with_input_schema(serde_json::json!({"type": "object"}))
        };
        let catalog = LlmProfileAbilityCatalog::from_system_abilities(vec![
            descriptor("conversation.send", "Send a prompt"),
            descriptor("skill.design", "Run a private skill"),
            descriptor("meta.list_abilities", "Device-owned metadata"),
        ]);

        let alice = descriptors_for_with_catalog(
            "easynet:///r/acme/agent/u1.alice",
            Some("claude-code"),
            &catalog,
        );
        let bob =
            descriptors_for_with_catalog("easynet:///r/acme/agent/u1.bob", Some("codex"), &catalog);

        assert_eq!(alice.len(), 2);
        assert_eq!(bob.len(), 2);
        assert!(alice
            .iter()
            .all(|descriptor| descriptor.owner_ura == "easynet:///r/acme/agent/u1.alice"));
        assert!(bob
            .iter()
            .all(|descriptor| descriptor.owner_ura == "easynet:///r/acme/agent/u1.bob"));
        assert!(alice
            .iter()
            .all(|descriptor| descriptor.name != "meta.list_abilities"));
        assert_eq!(
            alice
                .iter()
                .find(|descriptor| descriptor.name == "skill.design")
                .expect("skill descriptor")
                .visibility,
            crate::daemon::ability::descriptors::Visibility::Private
        );
    }
}
