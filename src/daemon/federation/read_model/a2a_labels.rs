//! A2A Agent-card projection over the committed local capability catalog.
//!
//! `AgentRegistry` contributes roster metadata only. Ability identity, schema,
//! description, transport, and visibility come exclusively from the immutable
//! `LocalAbilityPublicationSnapshot` captured from `AxonAbilityCatalog`.

use std::collections::BTreeSet;

use serde_json::{json, Value};

use crate::daemon::ability::catalog::LocalAbilityPublicationSnapshot;
use crate::daemon::ability::descriptors::AbilityDescriptor;
use crate::daemon::ability::CallMode;
use crate::daemon::persistence::agent_registry::AgentRegistry;

pub const A2A_SCHEMA_VERSION: &str = "v2";

/// Build the v2 `{"agents": [...]}` envelope from committed live rows.
///
/// A registered Agent with no committed RPC ability is intentionally absent:
/// advertising a roster-only card would create a visible-but-undispatchable
/// product state. Stream-only rows are also omitted because the current A2A
/// `send_task` bridge is unary.
pub fn build_agents_envelope(
    registry: &AgentRegistry,
    publication: &LocalAbilityPublicationSnapshot,
) -> Value {
    let agents = registry
        .agents
        .iter()
        .filter_map(|(name, entry)| {
            let mut seen = BTreeSet::new();
            let skills = publication
                .hosted_agent_descriptors(name)
                .into_iter()
                .filter(|descriptor| descriptor.call_mode() == CallMode::Rpc)
                .filter(|descriptor| {
                    !matches!(descriptor.public_name().as_str(), "discover" | "invoke")
                })
                .filter_map(|descriptor| {
                    let public_name = descriptor.public_name();
                    seen.insert(public_name.clone())
                        .then(|| skill_from_descriptor(name, public_name, descriptor))
                })
                .collect::<Vec<_>>();
            if skills.is_empty() {
                return None;
            }

            Some(json!({
                "a2a_schema_version": A2A_SCHEMA_VERSION,
                "description": entry.label,
                "model": entry.model,
                "name": name,
                "runtime": entry.agent_type.to_string(),
                "skills": skills,
            }))
        })
        .collect::<Vec<_>>();
    json!({"agents": agents})
}

fn skill_from_descriptor(
    agent_name: &str,
    public_name: String,
    descriptor: AbilityDescriptor,
) -> Value {
    let description = if descriptor.description.is_empty() {
        public_name.clone()
    } else {
        descriptor.description.clone()
    };
    let input_schema = descriptor.input_schema().clone();
    let output_schema = descriptor.output_receipt_schema().clone();
    let timeout_seconds = descriptor
        .metadata
        .get("timeout_seconds")
        .and_then(|value| value.parse::<u64>().ok())
        .map(Value::from)
        .unwrap_or(Value::Null);
    json!({
        "description": description,
        "input_schema": input_schema,
        "name": format!("{agent_name}.{public_name}"),
        "output_schema": output_schema,
        "timeout_seconds": timeout_seconds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::agent::spec::RuntimeKind;
    use crate::daemon::ability::descriptors::{AbilityDescriptor, Visibility};
    use crate::daemon::persistence::agent_registry::AgentEntry;

    fn registry(names: &[(&str, RuntimeKind)]) -> AgentRegistry {
        let mut registry = AgentRegistry::default();
        for (name, agent_type) in names {
            registry
                .agents
                .insert((*name).to_string(), AgentEntry::new(*agent_type, None));
        }
        registry
    }

    fn descriptor(agent: &str, verb: &str, mode: CallMode) -> AbilityDescriptor {
        let owner = crate::core::ura::agent_ura("test", "user", agent);
        AbilityDescriptor::new(
            verb,
            owner,
            Visibility::Scoped,
            crate::daemon::ability::descriptors::AdmissionAction::Invoke,
        )
        .unwrap()
        .with_call_mode(mode)
        .with_description(format!("{agent} {verb}"))
        .with_input_schema(json!({
            "type": "object",
            "properties": {"value": {"type": "string"}}
        }))
    }

    #[test]
    fn empty_publication_emits_no_roster_only_agents() {
        let envelope = build_agents_envelope(
            &registry(&[("alice", RuntimeKind::ClaudeCode)]),
            &LocalAbilityPublicationSnapshot::default(),
        );
        assert_eq!(envelope, json!({"agents": []}));
    }

    #[test]
    fn skills_are_projected_only_from_live_rpc_descriptors() {
        let publication = LocalAbilityPublicationSnapshot::from_descriptors(vec![
            descriptor("alice", "chat", CallMode::Rpc),
            descriptor("alice", "discover", CallMode::Rpc),
            descriptor("alice", "invoke", CallMode::Rpc),
            descriptor("alice", "events", CallMode::Stream),
        ]);
        let envelope = build_agents_envelope(
            &registry(&[("alice", RuntimeKind::ClaudeCode)]),
            &publication,
        );
        let skills = envelope["agents"][0]["skills"].as_array().unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0]["name"], "alice.chat");
        assert_eq!(skills[0]["description"], "alice chat");
        assert_eq!(skills[0]["input_schema"]["type"], "object");
    }

    #[test]
    fn same_public_name_stays_scoped_to_each_live_owner() {
        let publication = LocalAbilityPublicationSnapshot::from_descriptors(vec![
            descriptor("alice", "chat", CallMode::Rpc),
            descriptor("bob", "chat", CallMode::Rpc),
        ]);
        let envelope = build_agents_envelope(
            &registry(&[
                ("alice", RuntimeKind::ClaudeCode),
                ("bob", RuntimeKind::Codex),
            ]),
            &publication,
        );
        assert_eq!(envelope["agents"][0]["skills"][0]["name"], "alice.chat");
        assert_eq!(envelope["agents"][1]["skills"][0]["name"], "bob.chat");
    }

    #[test]
    fn projection_is_deterministic() {
        let publication = LocalAbilityPublicationSnapshot::from_descriptors(vec![
            descriptor("alice", "search", CallMode::Rpc),
            descriptor("alice", "chat", CallMode::Rpc),
        ]);
        let roster = registry(&[("alice", RuntimeKind::ClaudeCode)]);
        assert_eq!(
            build_agents_envelope(&roster, &publication),
            build_agents_envelope(&roster, &publication)
        );
        assert_eq!(
            build_agents_envelope(&roster, &publication)["agents"][0]["skills"][0]["name"],
            "alice.chat"
        );
    }
}
