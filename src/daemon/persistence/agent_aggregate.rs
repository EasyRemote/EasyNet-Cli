//! File: `src/daemon/persistence/agent_aggregate.rs`
//! Description: Read-side aggregate snapshots for hosted-Agent state.
//!
//! Protocol responsibility: expose a single per-call view of the durable Agent
//! registry and hosted-Agent identity index for public read projections.
//!
//! Implementation approach: load each persistence file exactly once and return
//! an immutable snapshot. This module performs no mutation and owns no cache.
//!
//! Usage contract: callers that need both registry rows and hosted Agent URAs
//! must request an aggregate snapshot instead of independently loading files.
//!
//! Architectural position: daemon persistence/domain layer below Agent read
//! abilities and beside lifecycle mutation persistence.

use std::collections::{BTreeMap, BTreeSet};

use crate::core::agent::id::{AgentId, DEFAULT_TENANT};

use super::agent_registry::{self, AgentEntry, AgentRegistry};
use super::local_agents::{self, HostedAgentEntry, LocalAgentsFile};

#[derive(Debug, thiserror::Error)]
pub(crate) enum AgentAggregateSnapshotLoadError {
    #[error("load Agent registry projection: {source:#}")]
    RegistryUnreadable { source: anyhow::Error },
    #[error("load hosted-Agent identity projection: {source:#}")]
    IdentityUnreadable { source: anyhow::Error },
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum HostedAgentNameLookupError {
    #[error(
        "hosted Agent name {name:?} is ambiguous across profiles {first_profile:?} and {second_profile:?}"
    )]
    Ambiguous {
        name: String,
        first_profile: String,
        second_profile: String,
    },
    #[error("hosted Agent {name:?} has invalid URA {agent_ura:?}: {reason}")]
    InvalidUra {
        name: String,
        agent_ura: String,
        reason: String,
    },
    #[error("hosted Agent {name:?} resolved to non-Agent URA {agent_ura:?}")]
    NonAgentUra { name: String, agent_ura: String },
}

#[derive(Debug, Clone)]
pub(crate) struct AgentAggregateSnapshot {
    pub(crate) registry: AgentRegistry,
    pub(crate) local_agents: LocalAgentsFile,
}

impl AgentAggregateSnapshot {
    pub(crate) fn new(registry: AgentRegistry, local_agents: LocalAgentsFile) -> Self {
        Self {
            registry,
            local_agents,
        }
    }

    pub(crate) fn has_registered_agent(&self, agent: &str) -> bool {
        self.registry.agents.contains_key(agent)
    }

    pub(crate) fn registered_agents(&self) -> impl Iterator<Item = (&str, &AgentEntry)> {
        self.registry
            .agents
            .iter()
            .map(|(name, entry)| (name.as_str(), entry))
    }

    pub(crate) fn registered_agent_surface_names(&self) -> BTreeSet<String> {
        let mut registered = BTreeSet::new();
        for raw_key in self.registry.agents.keys() {
            if let Ok(id) = AgentId::parse(raw_key) {
                registered.insert(id.name.clone());
                if id.tenant != DEFAULT_TENANT {
                    registered.insert(format!("{}/{}", id.tenant, id.name));
                } else {
                    registered.insert(format!("{}/{}", DEFAULT_TENANT, id.name));
                }
            }
        }
        registered
    }

    pub(crate) fn host_device_agent_ura(&self) -> &str {
        &self.local_agents.host_device_agent_ura
    }

    pub(crate) fn hosted_llm_agent_identity(&self, agent: &str) -> HostedLlmAgentIdentity<'_> {
        let mut matches = self
            .local_agents
            .hosted_agents
            .iter()
            .filter(|entry| entry.profile == "llm" && entry.name == agent);
        let Some(identity) = matches.next() else {
            return HostedLlmAgentIdentity::Missing;
        };
        if matches.next().is_some() {
            return HostedLlmAgentIdentity::Ambiguous;
        }
        HostedLlmAgentIdentity::Present(identity)
    }

    pub(crate) fn hosted_llm_agent_ura(&self, agent: &str) -> Option<&str> {
        match self.hosted_llm_agent_identity(agent) {
            HostedLlmAgentIdentity::Present(identity) => Some(identity.agent_ura.as_str()),
            HostedLlmAgentIdentity::Missing | HostedLlmAgentIdentity::Ambiguous => None,
        }
    }

    pub(crate) fn hosted_agent_ura_by_name(
        &self,
        name: &str,
    ) -> Result<Option<&str>, HostedAgentNameLookupError> {
        let mut matches = self
            .local_agents
            .hosted_agents
            .iter()
            .filter(|entry| entry.name == name);
        let Some(entry) = matches.next() else {
            return Ok(None);
        };
        if let Some(other) = matches.next() {
            return Err(HostedAgentNameLookupError::Ambiguous {
                name: name.to_string(),
                first_profile: entry.profile.clone(),
                second_profile: other.profile.clone(),
            });
        }
        let parsed = crate::core::ura::parse_ura(&entry.agent_ura).map_err(|error| {
            HostedAgentNameLookupError::InvalidUra {
                name: name.to_string(),
                agent_ura: entry.agent_ura.clone(),
                reason: error.to_string(),
            }
        })?;
        if parsed.kind != crate::core::ura::URAKind::Agent {
            return Err(HostedAgentNameLookupError::NonAgentUra {
                name: name.to_string(),
                agent_ura: entry.agent_ura.clone(),
            });
        }
        Ok(Some(entry.agent_ura.as_str()))
    }

    pub(crate) fn has_hosted_llm_agent_identity(&self, agent: &str) -> bool {
        self.local_agents
            .hosted_agents
            .iter()
            .any(|entry| entry.profile == "llm" && entry.name == agent)
    }

    pub(crate) fn local_target_projection(&self) -> AgentLocalTargetProjection {
        AgentLocalTargetProjection {
            hosted_agent_targets: self
                .local_agents
                .hosted_agents
                .iter()
                .filter_map(|entry| HostedAgentTarget::parse(&entry.agent_ura))
                .collect(),
            registered_agent_ids: self.registry.agents.keys().cloned().collect(),
        }
    }

    pub(crate) fn hosted_agent_placements(&self) -> AgentHostedPlacementProjection {
        let host_device_ura = self.local_agents.host_device_agent_ura.trim();
        if host_device_ura.is_empty() {
            return AgentHostedPlacementProjection::default();
        }
        let host_node_id = device_node_id_from_device_ura(host_device_ura);
        AgentHostedPlacementProjection {
            by_agent_ura: self
                .local_agents
                .hosted_agents
                .iter()
                .filter_map(|entry| {
                    let agent_ura = entry.agent_ura.trim();
                    HostedAgentTarget::parse(agent_ura)?;
                    Some((
                        agent_ura.to_string(),
                        AgentHostedPlacement {
                            agent_ura: agent_ura.to_string(),
                            host_device_ura: host_device_ura.to_string(),
                            host_node_id: host_node_id.clone(),
                        },
                    ))
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum HostedLlmAgentIdentity<'a> {
    Missing,
    Present(&'a HostedAgentEntry),
    Ambiguous,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AgentLocalTargetProjection {
    pub(crate) hosted_agent_targets: BTreeSet<HostedAgentTarget>,
    pub(crate) registered_agent_ids: BTreeSet<String>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct HostedAgentTarget {
    pub(crate) realm: String,
    pub(crate) user_id: String,
    pub(crate) agent_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AgentHostedPlacementProjection {
    pub(crate) by_agent_ura: BTreeMap<String, AgentHostedPlacement>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentHostedPlacement {
    pub(crate) agent_ura: String,
    pub(crate) host_device_ura: String,
    pub(crate) host_node_id: Option<String>,
}

impl HostedAgentTarget {
    pub(crate) fn parse(agent_ura: &str) -> Option<Self> {
        let parsed = crate::core::ura::parse_ura(agent_ura).ok()?;
        if !matches!(parsed.kind, crate::core::ura::URAKind::Agent) {
            return None;
        }
        let realm = parsed.realm.clone();
        let (user_id, agent_id) = parsed.agent_ids()?;
        if realm.is_empty() || user_id.is_empty() || agent_id.is_empty() {
            return None;
        }
        Some(Self {
            realm,
            user_id: user_id.to_string(),
            agent_id: agent_id.to_string(),
        })
    }
}

fn device_node_id_from_device_ura(device_ura: &str) -> Option<String> {
    crate::core::ura::parse_ura(device_ura)
        .ok()
        .filter(|parsed| parsed.kind == crate::core::ura::URAKind::Device)
        .and_then(|parsed| parsed.device_id().map(str::to_string))
}

pub(crate) struct AgentAggregateRepository;

impl AgentAggregateRepository {
    pub(crate) fn load_snapshot() -> anyhow::Result<AgentAggregateSnapshot> {
        Self::try_load_snapshot().map_err(Into::into)
    }

    pub(crate) fn try_load_snapshot()
    -> Result<AgentAggregateSnapshot, AgentAggregateSnapshotLoadError> {
        let registry = agent_registry::load_agents()
            .map_err(|source| AgentAggregateSnapshotLoadError::RegistryUnreadable { source })?;
        let local_agents = local_agents::load()
            .map_err(|source| AgentAggregateSnapshotLoadError::IdentityUnreadable { source })?;
        Ok(AgentAggregateSnapshot::new(registry, local_agents))
    }
}

#[cfg(test)]
mod tests {
    use super::agent_registry::{AgentEntry, AgentType};
    use super::*;

    fn hosted_agent(profile: &str, name: &str, agent_ura: &str) -> HostedAgentEntry {
        HostedAgentEntry {
            profile: profile.to_string(),
            name: name.to_string(),
            agent_ura: agent_ura.to_string(),
            signing_authority: "hosted_by:easynet:///r/acme/agent/device".to_string(),
            first_seen_at: "2026-07-16T00:00:00Z".to_string(),
        }
    }

    fn snapshot(entries: Vec<HostedAgentEntry>) -> AgentAggregateSnapshot {
        AgentAggregateSnapshot::new(
            AgentRegistry::default(),
            LocalAgentsFile {
                host_device_agent_ura: "easynet:///r/acme/agent/device".to_string(),
                hosted_agents: entries,
            },
        )
    }

    #[test]
    fn hosted_llm_agent_identity_requires_llm_profile() {
        let snapshot = snapshot(vec![hosted_agent(
            "mcp",
            "claude",
            "easynet:///r/acme/agent/u1.mcp",
        )]);

        assert!(matches!(
            snapshot.hosted_llm_agent_identity("claude"),
            HostedLlmAgentIdentity::Missing
        ));
        assert!(!snapshot.has_hosted_llm_agent_identity("claude"));
    }

    #[test]
    fn hosted_llm_agent_identity_resolves_single_identity() {
        let snapshot = snapshot(vec![hosted_agent(
            "llm",
            "claude",
            "easynet:///r/acme/agent/u1.claude",
        )]);

        let HostedLlmAgentIdentity::Present(identity) =
            snapshot.hosted_llm_agent_identity("claude")
        else {
            panic!("expected one hosted llm identity");
        };
        assert_eq!(identity.agent_ura, "easynet:///r/acme/agent/u1.claude");
        assert!(snapshot.has_hosted_llm_agent_identity("claude"));
    }

    #[test]
    fn hosted_llm_agent_identity_detects_duplicate_llm_rows() {
        let snapshot = snapshot(vec![
            hosted_agent("llm", "claude", "easynet:///r/acme/agent/u1.claude"),
            hosted_agent("llm", "claude", "easynet:///r/acme/agent/u1.claude2"),
        ]);

        assert!(matches!(
            snapshot.hosted_llm_agent_identity("claude"),
            HostedLlmAgentIdentity::Ambiguous
        ));
        assert!(snapshot.has_hosted_llm_agent_identity("claude"));
    }

    #[test]
    fn hosted_llm_agent_ura_requires_one_unambiguous_identity() {
        let missing = snapshot(vec![hosted_agent(
            "mcp",
            "claude",
            "easynet:///r/acme/agent/u1.mcp",
        )]);
        assert_eq!(missing.hosted_llm_agent_ura("claude"), None);

        let single = snapshot(vec![hosted_agent(
            "llm",
            "claude",
            "easynet:///r/acme/agent/u1.claude",
        )]);
        assert_eq!(
            single.hosted_llm_agent_ura("claude"),
            Some("easynet:///r/acme/agent/u1.claude")
        );

        let duplicate = snapshot(vec![
            hosted_agent("llm", "claude", "easynet:///r/acme/agent/u1.claude"),
            hosted_agent("llm", "claude", "easynet:///r/acme/agent/u1.claude2"),
        ]);
        assert_eq!(duplicate.hosted_llm_agent_ura("claude"), None);
    }

    #[test]
    fn registered_agent_surface_names_include_bare_and_tenant_forms() {
        let mut registry = AgentRegistry::default();
        registry.agents.insert(
            "claude".to_string(),
            AgentEntry::new(AgentType::ClaudeCode, None),
        );
        registry.agents.insert(
            "research/codex".to_string(),
            AgentEntry::new(AgentType::Codex, None),
        );
        let snapshot = AgentAggregateSnapshot::new(registry, LocalAgentsFile::default());

        let names = snapshot.registered_agent_surface_names();

        assert!(names.contains("claude"));
        assert!(names.contains("default/claude"));
        assert!(names.contains("codex"));
        assert!(names.contains("research/codex"));
    }

    #[test]
    fn hosted_agent_name_lookup_resolves_exact_agent_ura() {
        let snapshot = snapshot(vec![hosted_agent(
            "llm",
            "claude",
            "easynet:///r/acme/agent/u1.claude",
        )]);

        assert_eq!(
            snapshot.hosted_agent_ura_by_name("claude").unwrap(),
            Some("easynet:///r/acme/agent/u1.claude")
        );
        assert_eq!(snapshot.hosted_agent_ura_by_name("missing").unwrap(), None);
    }

    #[test]
    fn hosted_agent_name_lookup_detects_ambiguous_display_names() {
        let snapshot = snapshot(vec![
            hosted_agent("llm", "same", "easynet:///r/acme/agent/u1.same"),
            hosted_agent("mcp", "same", "easynet:///r/acme/agent/u1.same-mcp"),
        ]);

        let error = snapshot
            .hosted_agent_ura_by_name("same")
            .expect_err("duplicate display name must be ambiguous");

        assert!(matches!(
            error,
            HostedAgentNameLookupError::Ambiguous { .. }
        ));
    }

    #[test]
    fn hosted_agent_name_lookup_rejects_invalid_and_non_agent_uras() {
        let invalid = snapshot(vec![hosted_agent("llm", "bad", "not-a-ura")]);
        assert!(matches!(
            invalid
                .hosted_agent_ura_by_name("bad")
                .expect_err("invalid URA must fail"),
            HostedAgentNameLookupError::InvalidUra { .. }
        ));

        let non_agent = snapshot(vec![hosted_agent(
            "llm",
            "device",
            "easynet:///r/acme/device/dev-1",
        )]);
        assert!(matches!(
            non_agent
                .hosted_agent_ura_by_name("device")
                .expect_err("non-Agent URA must fail"),
            HostedAgentNameLookupError::NonAgentUra { .. }
        ));
    }

    #[test]
    fn local_target_projection_parses_hosted_targets_and_registered_agent_ids() {
        let mut registry = AgentRegistry::default();
        registry.agents.insert(
            "claude".to_string(),
            AgentEntry::new(AgentType::ClaudeCode, None),
        );
        registry
            .agents
            .insert("codex".to_string(), AgentEntry::new(AgentType::Codex, None));
        let snapshot = AgentAggregateSnapshot::new(
            registry,
            LocalAgentsFile {
                host_device_agent_ura: "easynet:///r/acme/agent/device".to_string(),
                hosted_agents: vec![
                    hosted_agent("llm", "claude", "easynet:///r/acme/agent/u1.claude"),
                    hosted_agent("llm", "malformed", "not-a-ura"),
                    hosted_agent("consent", "default", "easynet:///r/acme/device/dev-1"),
                ],
            },
        );

        let projection = snapshot.local_target_projection();

        assert!(
            projection
                .hosted_agent_targets
                .contains(&HostedAgentTarget {
                    realm: "acme".to_string(),
                    user_id: "u1".to_string(),
                    agent_id: "claude".to_string(),
                })
        );
        assert_eq!(projection.hosted_agent_targets.len(), 1);
        assert!(projection.registered_agent_ids.contains("claude"));
        assert!(projection.registered_agent_ids.contains("codex"));
    }

    #[test]
    fn hosted_agent_placements_project_valid_agent_hosts() {
        let snapshot = AgentAggregateSnapshot::new(
            AgentRegistry::default(),
            LocalAgentsFile {
                host_device_agent_ura: "easynet:///r/acme/device/dev-1".to_string(),
                hosted_agents: vec![
                    hosted_agent("llm", "claude", "easynet:///r/acme/agent/u1.claude"),
                    hosted_agent("llm", "bad", "not-a-ura"),
                    hosted_agent("consent", "default", "easynet:///r/acme/device/dev-1"),
                ],
            },
        );

        let projection = snapshot.hosted_agent_placements();

        assert_eq!(projection.by_agent_ura.len(), 1);
        let placement = projection
            .by_agent_ura
            .get("easynet:///r/acme/agent/u1.claude")
            .expect("valid hosted Agent placement");
        assert_eq!(placement.agent_ura, "easynet:///r/acme/agent/u1.claude");
        assert_eq!(placement.host_device_ura, "easynet:///r/acme/device/dev-1");
        assert_eq!(placement.host_node_id.as_deref(), Some("dev-1"));
    }

    #[test]
    fn hosted_agent_placements_fail_closed_without_host_device() {
        let projection = AgentAggregateSnapshot::new(
            AgentRegistry::default(),
            LocalAgentsFile {
                host_device_agent_ura: String::new(),
                hosted_agents: vec![hosted_agent(
                    "llm",
                    "claude",
                    "easynet:///r/acme/agent/u1.claude",
                )],
            },
        )
        .hosted_agent_placements();

        assert!(projection.by_agent_ura.is_empty());
    }
}
