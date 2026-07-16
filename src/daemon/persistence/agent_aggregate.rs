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

use super::agent_registry::{self, AgentRegistry};
use super::local_agents::{self, HostedAgentEntry, LocalAgentsFile};

#[derive(Debug, thiserror::Error)]
pub(crate) enum AgentAggregateSnapshotLoadError {
    #[error("load Agent registry projection: {source:#}")]
    RegistryUnreadable { source: anyhow::Error },
    #[error("load hosted-Agent identity projection: {source:#}")]
    IdentityUnreadable { source: anyhow::Error },
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

    pub(crate) fn has_hosted_llm_agent_identity(&self, agent: &str) -> bool {
        self.local_agents
            .hosted_agents
            .iter()
            .any(|entry| entry.profile == "llm" && entry.name == agent)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum HostedLlmAgentIdentity<'a> {
    Missing,
    Present(&'a HostedAgentEntry),
    Ambiguous,
}

pub(crate) struct AgentAggregateRepository;

impl AgentAggregateRepository {
    pub(crate) fn load_snapshot() -> anyhow::Result<AgentAggregateSnapshot> {
        Self::try_load_snapshot().map_err(Into::into)
    }

    pub(crate) fn try_load_snapshot(
    ) -> Result<AgentAggregateSnapshot, AgentAggregateSnapshotLoadError> {
        let registry = agent_registry::load_agents()
            .map_err(|source| AgentAggregateSnapshotLoadError::RegistryUnreadable { source })?;
        let local_agents = local_agents::load()
            .map_err(|source| AgentAggregateSnapshotLoadError::IdentityUnreadable { source })?;
        Ok(AgentAggregateSnapshot::new(registry, local_agents))
    }
}

#[cfg(test)]
mod tests {
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
}
