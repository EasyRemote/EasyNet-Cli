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
use std::path::{Path, PathBuf};

use crate::core::agent::id::{AgentId, DEFAULT_TENANT};
use crate::core::agent::spec::RuntimeKind;

use super::agent_registry::{self, AgentEntry, AgentRegistry};
use super::local_agents::{
    self, HostedAgentEntry, LocalAgentsFile, LocalHostedAgentIdentityAggregate,
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum AgentAggregateSnapshotLoadError {
    #[error("load Agent registry projection: {source:#}")]
    RegistryUnreadable { source: anyhow::Error },
    #[error("load hosted-Agent identity projection: {source:#}")]
    IdentityUnreadable { source: anyhow::Error },
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum AgentRegistryProjectionLoadError {
    #[error("load Agent registry projection: {source:#}")]
    RegistryUnreadable { source: anyhow::Error },
}

impl AgentRegistryProjectionLoadError {
    pub(crate) fn into_source_or_self(self) -> anyhow::Error {
        match self {
            Self::RegistryUnreadable { source } => source,
        }
    }
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
        let Ok(registry_key) = AgentId::parse(agent).map(|agent_id| agent_id.to_string()) else {
            return false;
        };
        self.registry.agents.contains_key(&registry_key)
    }

    pub(crate) fn registered_agents(&self) -> impl Iterator<Item = (&str, &AgentEntry)> {
        self.registry
            .agents
            .iter()
            .map(|(name, entry)| (name.as_str(), entry))
    }

    pub(crate) fn registered_agent_names(
        &self,
    ) -> Result<BTreeSet<String>, AgentRegisteredIdentityProjectionError> {
        let mut names = BTreeSet::new();
        for raw_key in self.registry.agents.keys() {
            let id = parse_registered_agent_key(raw_key)?;
            names.insert(id.name);
        }
        Ok(names)
    }

    pub(crate) fn registered_agent_registry_projection(&self) -> AgentRegistry {
        self.registry.clone()
    }

    pub(crate) fn registered_agent_workspace(
        &self,
        owner_id: &str,
        operation: &str,
    ) -> Result<AgentRegisteredWorkspace, AgentRegisteredWorkspaceLookupError> {
        self.registered_agent(owner_id, operation)
            .map(AgentRegisteredAgent::into_workspace)
    }

    pub(crate) fn registered_agent_runtime_projection(
        &self,
        owner_id: &str,
    ) -> Option<AgentRegisteredRuntimeProjection> {
        let registry_key = AgentId::parse(owner_id).ok()?.to_string();
        self.registry
            .agents
            .get(&registry_key)
            .cloned()
            .map(AgentRegisteredRuntimeProjection::new)
    }

    fn registered_agent(
        &self,
        owner_id: &str,
        operation: &str,
    ) -> Result<AgentRegisteredAgent, AgentRegisteredWorkspaceLookupError> {
        AgentRegisteredAgent::from_registry(&self.registry, owner_id, operation)
    }

    pub(crate) fn registered_agent_surface_names(
        &self,
    ) -> Result<BTreeSet<String>, AgentRegisteredIdentityProjectionError> {
        let mut registered = BTreeSet::new();
        for raw_key in self.registry.agents.keys() {
            let id = parse_registered_agent_key(raw_key)?;
            registered.insert(id.name.clone());
            if id.tenant != DEFAULT_TENANT {
                registered.insert(format!("{}/{}", id.tenant, id.name));
            } else {
                registered.insert(format!("{}/{}", DEFAULT_TENANT, id.name));
            }
        }
        Ok(registered)
    }

    pub(crate) fn host_device_ura(&self) -> &str {
        &self.local_agents.host_device_ura
    }

    #[cfg(test)]
    pub(crate) fn hosted_identity_status(&self) -> AgentHostedIdentityStatus {
        AgentHostedIdentityStatus::from_local_agents(&self.local_agents)
    }

    pub(crate) fn hosted_skill_owner_projection(&self) -> AgentHostedSkillOwnerProjection {
        AgentHostedSkillOwnerProjection::from_local_agents(&self.local_agents)
    }

    pub(crate) fn hosted_llm_agent_identity(&self, agent: &str) -> HostedLlmAgentIdentity<'_> {
        hosted_llm_agent_identity(&self.local_agents, agent)
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
        Ok(self
            .hosted_agent_identity_by_name(name)?
            .map(|identity| identity.agent_ura))
    }

    pub(crate) fn hosted_agent_identity_by_name(
        &self,
        name: &str,
    ) -> Result<Option<HostedAgentIdentityProjection<'_>>, HostedAgentNameLookupError> {
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
        validate_hosted_agent_name_identity(name, entry)?;
        Ok(Some(HostedAgentIdentityProjection::from_entry(entry)))
    }

    pub(crate) fn hosted_agent_identity_by_ura(
        &self,
        agent_ura: &str,
    ) -> Option<HostedAgentIdentityProjection<'_>> {
        self.local_agents
            .hosted_agents
            .iter()
            .find(|entry| entry.agent_ura == agent_ura)
            .map(HostedAgentIdentityProjection::from_entry)
    }

    pub(crate) fn has_hosted_llm_agent_identity(&self, agent: &str) -> bool {
        self.local_agents
            .hosted_agents
            .iter()
            .any(|entry| entry.profile == "llm" && entry.name == agent)
    }

    pub(crate) fn local_target_projection(
        &self,
    ) -> Result<AgentLocalTargetProjection, AgentLocalTargetProjectionError> {
        let hosted_agent_targets = self
            .local_agents
            .hosted_agents
            .iter()
            .map(HostedAgentTarget::from_entry)
            .collect::<Result<BTreeSet<_>, _>>()?;
        Ok(AgentLocalTargetProjection {
            hosted_agent_targets,
            registered_agent_ids: self.registered_agent_surface_names()?,
        })
    }

    pub(crate) fn hosted_agent_placements(
        &self,
    ) -> Result<AgentHostedPlacementProjection, HostedAgentIdentityProjectionError> {
        let host_device_ura = self.local_agents.host_device_ura.trim();
        if host_device_ura.is_empty() {
            return Ok(AgentHostedPlacementProjection::default());
        }
        let host_node_id = device_node_id_from_device_ura(host_device_ura);
        let by_agent_ura = self
            .local_agents
            .hosted_agents
            .iter()
            .map(|entry| {
                HostedAgentTarget::from_entry(entry)?;
                let agent_ura = entry.agent_ura.trim().to_string();
                Ok((
                    agent_ura.clone(),
                    AgentHostedPlacement {
                        agent_ura,
                        host_device_ura: host_device_ura.to_string(),
                        host_node_id: host_node_id.clone(),
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, HostedAgentIdentityProjectionError>>()?;
        Ok(AgentHostedPlacementProjection { by_agent_ura })
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum AgentRegisteredWorkspaceLookupError {
    #[error(
        "owner_agent_id {owner_id:?} is not registered (registered agents: {:?})",
        registered_agent_ids
    )]
    Missing {
        owner_id: String,
        registered_agent_ids: Vec<String>,
    },
    #[error("owner_agent_id {owner_id:?} is invalid for {operation}: {reason}")]
    InvalidOwnerId {
        owner_id: String,
        operation: String,
        reason: String,
    },
    #[error("registered Agent workspace is invalid: {source:#}")]
    InvalidWorkspace { source: anyhow::Error },
}

impl AgentRegisteredWorkspaceLookupError {
    pub(crate) fn into_source_or_self(self) -> anyhow::Error {
        match self {
            Self::InvalidWorkspace { source } => source,
            error => error.into(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum AgentRegisteredAgentLoadError {
    #[error("load Agent registry projection: {source:#}")]
    RegistryUnreadable { source: anyhow::Error },
    #[error(transparent)]
    Lookup(#[from] AgentRegisteredWorkspaceLookupError),
}

impl AgentRegisteredAgentLoadError {
    pub(crate) fn into_source_or_self(self) -> anyhow::Error {
        match self {
            Self::RegistryUnreadable { source } => source,
            Self::Lookup(error) => error.into_source_or_self(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AgentRegisteredAgent {
    entry: AgentEntry,
    workspace: AgentRegisteredWorkspace,
}

impl AgentRegisteredAgent {
    fn from_registry(
        registry: &AgentRegistry,
        owner_id: &str,
        operation: &str,
    ) -> Result<Self, AgentRegisteredWorkspaceLookupError> {
        let registry_key = AgentId::parse(owner_id)
            .map_err(
                |error| AgentRegisteredWorkspaceLookupError::InvalidOwnerId {
                    owner_id: owner_id.to_string(),
                    operation: operation.to_string(),
                    reason: error.to_string(),
                },
            )?
            .to_string();
        let entry = registry.agents.get(&registry_key).cloned().ok_or_else(|| {
            AgentRegisteredWorkspaceLookupError::Missing {
                owner_id: owner_id.to_string(),
                registered_agent_ids: registry.agents.keys().cloned().collect(),
            }
        })?;
        let workspace = AgentRegisteredWorkspace::from_entry(&entry, owner_id, operation)?;
        Ok(Self { entry, workspace })
    }

    pub(crate) fn entry(&self) -> &AgentEntry {
        &self.entry
    }

    pub(crate) fn workspace(&self) -> &AgentRegisteredWorkspace {
        &self.workspace
    }

    fn into_workspace(self) -> AgentRegisteredWorkspace {
        self.workspace
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AgentRegisteredRuntimeProjection {
    entry: AgentEntry,
}

impl AgentRegisteredRuntimeProjection {
    fn new(entry: AgentEntry) -> Self {
        Self { entry }
    }

    pub(crate) fn entry(&self) -> &AgentEntry {
        &self.entry
    }

    pub(crate) fn ability_manifest_path(&self, ability: &str) -> Option<PathBuf> {
        self.entry.root_path.as_ref().map(|root| {
            root.join("abilities")
                .join(format!("{ability}.ability.toml"))
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentRegisteredWorkspace {
    root_path: PathBuf,
    skill_layout: AgentSkillLayout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSkillLayout {
    ClaudeCode,
    Codex,
    External,
}

impl AgentSkillLayout {
    fn from_agent_type(agent_type: RuntimeKind) -> Self {
        match agent_type {
            RuntimeKind::ClaudeCode => Self::ClaudeCode,
            RuntimeKind::Codex | RuntimeKind::CodexAppServer => Self::Codex,
            RuntimeKind::External => Self::External,
        }
    }
}

impl AgentRegisteredWorkspace {
    fn from_entry(
        entry: &AgentEntry,
        owner_id: &str,
        operation: &str,
    ) -> Result<Self, AgentRegisteredWorkspaceLookupError> {
        Ok(Self {
            root_path: entry
                .required_root_path(owner_id, operation)
                .map_err(
                    |source| AgentRegisteredWorkspaceLookupError::InvalidWorkspace { source },
                )?,
            skill_layout: AgentSkillLayout::from_agent_type(entry.agent_type),
        })
    }

    pub(crate) fn root_path(&self) -> &Path {
        &self.root_path
    }

    pub(crate) fn skill_layout(&self) -> AgentSkillLayout {
        self.skill_layout
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct HostedAgentIdentityProjection<'a> {
    #[cfg(test)]
    pub(crate) profile: &'a str,
    #[cfg(test)]
    pub(crate) name: &'a str,
    pub(crate) agent_ura: &'a str,
    pub(crate) signing_authority: &'a str,
}

impl<'a> HostedAgentIdentityProjection<'a> {
    fn from_entry(entry: &'a HostedAgentEntry) -> Self {
        Self {
            #[cfg(test)]
            profile: entry.profile.as_str(),
            #[cfg(test)]
            name: entry.name.as_str(),
            agent_ura: entry.agent_ura.as_str(),
            signing_authority: entry.signing_authority.as_str(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AgentHostedIdentitySnapshot {
    local_agents: LocalAgentsFile,
}

impl AgentHostedIdentitySnapshot {
    fn new(local_agents: LocalAgentsFile) -> Self {
        Self { local_agents }
    }

    pub(crate) fn host_descriptor_identity_projection(
        &self,
    ) -> AgentHostDescriptorIdentityProjection {
        AgentHostDescriptorIdentityProjection::from_local_agents(&self.local_agents)
    }

    pub(crate) fn hosted_identity_status(&self) -> AgentHostedIdentityStatus {
        AgentHostedIdentityStatus::from_local_agents(&self.local_agents)
    }

    pub(crate) fn hosted_llm_agent_ura(&self, agent: &str) -> Option<&str> {
        match hosted_llm_agent_identity(&self.local_agents, agent) {
            HostedLlmAgentIdentity::Present(identity) => Some(identity.agent_ura.as_str()),
            HostedLlmAgentIdentity::Missing | HostedLlmAgentIdentity::Ambiguous => None,
        }
    }

    pub(crate) fn hosted_agent_authority_roots(&self) -> anyhow::Result<Vec<String>> {
        let aggregate = LocalHostedAgentIdentityAggregate::validate(&self.local_agents)?;
        Ok(aggregate
            .hosted_agents()
            .iter()
            .map(|entry| entry.agent_ura.clone())
            .collect())
    }

    pub(crate) fn hosted_advertise_entries(
        &self,
        realm: &str,
        user_segment: &str,
    ) -> Vec<AgentHostedAdvertiseEntry> {
        let mut entries = Vec::new();
        let mut seen = BTreeSet::new();

        for hosted in &self.local_agents.hosted_agents {
            let agent_ura = hosted.agent_ura.trim();
            if agent_ura.is_empty() || agent_ura.contains("<unjoined>") {
                continue;
            }
            if !hosted_agent_belongs_to_runtime_user(agent_ura, realm, user_segment) {
                continue;
            }
            if seen.insert(agent_ura.to_string()) {
                entries.push(AgentHostedAdvertiseEntry::configured(agent_ura));
            }
        }

        entries
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentHostedAdvertiseEntry {
    agent_ura: String,
    short_label: String,
}

impl AgentHostedAdvertiseEntry {
    fn configured(agent_ura: &str) -> Self {
        Self {
            agent_ura: agent_ura.to_string(),
            short_label: hosted_agent_short_label(agent_ura),
        }
    }

    pub(crate) fn agent_ura(&self) -> &str {
        &self.agent_ura
    }

    pub(crate) fn short_label(&self) -> &str {
        &self.short_label
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AgentHostedSkillOwnerProjection {
    by_agent_name: BTreeMap<String, String>,
    owner_by_agent_ura: BTreeMap<String, String>,
}

impl AgentHostedSkillOwnerProjection {
    fn from_local_agents(local_agents: &LocalAgentsFile) -> Self {
        let mut by_agent_name = BTreeMap::new();
        let mut owner_by_agent_ura = BTreeMap::new();
        for entry in &local_agents.hosted_agents {
            by_agent_name.insert(entry.name.clone(), entry.agent_ura.clone());
            owner_by_agent_ura
                .entry(entry.agent_ura.clone())
                .or_insert_with(|| entry.name.clone());
        }
        Self {
            by_agent_name,
            owner_by_agent_ura,
        }
    }

    pub(crate) fn hosted_ura_for(&self, agent_name: &str) -> Option<&str> {
        self.by_agent_name.get(agent_name).map(String::as_str)
    }

    pub(crate) fn owner_name_for_agent_ura(&self, agent_ura: &str) -> Option<&str> {
        self.owner_by_agent_ura.get(agent_ura).map(String::as_str)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentHostDescriptorIdentityProjection {
    host_device_ura: Option<String>,
    llm_agent_uras: Vec<(String, String)>,
}

impl AgentHostDescriptorIdentityProjection {
    fn from_local_agents(local_agents: &LocalAgentsFile) -> Self {
        Self {
            host_device_ura: trimmed_nonempty(&local_agents.host_device_ura).map(str::to_string),
            llm_agent_uras: local_agents
                .hosted_agents
                .iter()
                .filter(|entry| entry.profile == "llm")
                .map(|entry| (entry.name.clone(), entry.agent_ura.clone()))
                .collect(),
        }
    }

    pub(crate) fn host_device_ura(&self) -> Option<&str> {
        self.host_device_ura.as_deref()
    }

    pub(crate) fn llm_agent_uras(&self) -> &[(String, String)] {
        &self.llm_agent_uras
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentHostedIdentityStatus {
    host_device_ura: Option<String>,
    hosted_agent_count: usize,
}

impl AgentHostedIdentityStatus {
    fn from_local_agents(local_agents: &LocalAgentsFile) -> Self {
        let host_device_ura = local_agents.host_device_ura.trim();
        Self {
            host_device_ura: (!host_device_ura.is_empty()).then(|| host_device_ura.to_string()),
            hosted_agent_count: local_agents.hosted_agents.len(),
        }
    }

    pub(crate) fn is_joined(&self) -> bool {
        self.host_device_ura.is_some()
    }

    pub(crate) fn host_device_ura(&self) -> Option<&str> {
        self.host_device_ura.as_deref()
    }

    pub(crate) fn hosted_agent_count(&self) -> usize {
        self.hosted_agent_count
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum HostedLlmAgentIdentity<'a> {
    Missing,
    Present(&'a HostedAgentEntry),
    Ambiguous,
}

fn hosted_llm_agent_identity<'a>(
    local_agents: &'a LocalAgentsFile,
    agent: &str,
) -> HostedLlmAgentIdentity<'a> {
    let mut matches = local_agents
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

fn hosted_agent_short_label(agent_ura: &str) -> String {
    crate::core::ura::parse_ura(agent_ura)
        .ok()
        .filter(|parsed| parsed.kind == crate::core::ura::URAKind::Agent)
        .and_then(|parsed| {
            parsed
                .agent_ids()
                .map(|(user_id, agent_id)| format!("{user_id}.{agent_id}"))
        })
        .unwrap_or_else(|| agent_ura.to_string())
}

fn hosted_agent_belongs_to_runtime_user(agent_ura: &str, realm: &str, user_segment: &str) -> bool {
    if realm.is_empty() || user_segment.is_empty() {
        return false;
    }
    let Ok(parsed) = crate::core::ura::parse_ura(agent_ura) else {
        return false;
    };
    if parsed.kind != crate::core::ura::URAKind::Agent || parsed.realm != realm {
        return false;
    }
    if parsed.device_agent_ids().is_some() {
        return false;
    }
    parsed
        .agent_ids()
        .is_some_and(|(owner_user_segment, _)| owner_user_segment == user_segment)
}

fn trimmed_nonempty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn validate_hosted_agent_name_identity(
    name: &str,
    entry: &HostedAgentEntry,
) -> Result<(), HostedAgentNameLookupError> {
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
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AgentLocalTargetProjection {
    pub(crate) hosted_agent_targets: BTreeSet<HostedAgentTarget>,
    pub(crate) registered_agent_ids: BTreeSet<String>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum AgentLocalTargetProjectionError {
    #[error(transparent)]
    HostedIdentity(#[from] HostedAgentIdentityProjectionError),
    #[error(transparent)]
    RegisteredIdentity(#[from] AgentRegisteredIdentityProjectionError),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum AgentRegisteredIdentityProjectionError {
    #[error("registered Agent registry key {registry_key:?} is not canonical: {reason}")]
    InvalidRegisteredAgentKey {
        registry_key: String,
        reason: String,
    },
}

fn parse_registered_agent_key(
    raw_key: &str,
) -> Result<AgentId, AgentRegisteredIdentityProjectionError> {
    AgentId::parse(raw_key).map_err(|error| {
        AgentRegisteredIdentityProjectionError::InvalidRegisteredAgentKey {
            registry_key: raw_key.to_string(),
            reason: error.to_string(),
        }
    })
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum HostedAgentIdentityProjectionError {
    #[error("hosted Agent {profile:?}/{name:?} has invalid Agent URA {agent_ura:?}: {reason}")]
    InvalidHostedAgentUra {
        profile: String,
        name: String,
        agent_ura: String,
        reason: String,
    },
    #[error("hosted Agent {profile:?}/{name:?} resolved to non-Agent URA {agent_ura:?}")]
    NonAgentHostedIdentity {
        profile: String,
        name: String,
        agent_ura: String,
    },
    #[error("hosted Agent {profile:?}/{name:?} has incomplete Agent identity {agent_ura:?}")]
    IncompleteHostedAgentIdentity {
        profile: String,
        name: String,
        agent_ura: String,
    },
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
        Self::from_ura(agent_ura).ok()
    }

    fn from_entry(entry: &HostedAgentEntry) -> Result<Self, HostedAgentIdentityProjectionError> {
        Self::from_ura(&entry.agent_ura).map_err(|error| match error {
            HostedAgentTargetParseError::InvalidUra { reason } => {
                HostedAgentIdentityProjectionError::InvalidHostedAgentUra {
                    profile: entry.profile.clone(),
                    name: entry.name.clone(),
                    agent_ura: entry.agent_ura.clone(),
                    reason,
                }
            }
            HostedAgentTargetParseError::NonAgentUra => {
                HostedAgentIdentityProjectionError::NonAgentHostedIdentity {
                    profile: entry.profile.clone(),
                    name: entry.name.clone(),
                    agent_ura: entry.agent_ura.clone(),
                }
            }
            HostedAgentTargetParseError::IncompleteAgentIdentity => {
                HostedAgentIdentityProjectionError::IncompleteHostedAgentIdentity {
                    profile: entry.profile.clone(),
                    name: entry.name.clone(),
                    agent_ura: entry.agent_ura.clone(),
                }
            }
        })
    }

    fn from_ura(agent_ura: &str) -> Result<Self, HostedAgentTargetParseError> {
        let parsed = crate::core::ura::parse_ura(agent_ura).map_err(|error| {
            HostedAgentTargetParseError::InvalidUra {
                reason: error.to_string(),
            }
        })?;
        if !matches!(parsed.kind, crate::core::ura::URAKind::Agent) {
            return Err(HostedAgentTargetParseError::NonAgentUra);
        }
        let realm = parsed.realm.clone();
        let (user_id, agent_id) = parsed
            .agent_ids()
            .ok_or(HostedAgentTargetParseError::IncompleteAgentIdentity)?;
        if realm.is_empty() || user_id.is_empty() || agent_id.is_empty() {
            return Err(HostedAgentTargetParseError::IncompleteAgentIdentity);
        }
        Ok(Self {
            realm,
            user_id: user_id.to_string(),
            agent_id: agent_id.to_string(),
        })
    }
}

#[derive(Debug)]
enum HostedAgentTargetParseError {
    InvalidUra { reason: String },
    NonAgentUra,
    IncompleteAgentIdentity,
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

    pub(crate) fn load_hosted_identity_snapshot() -> anyhow::Result<AgentHostedIdentitySnapshot> {
        Ok(AgentHostedIdentitySnapshot::new(
            Self::load_hosted_identity_projection()?,
        ))
    }

    pub(crate) fn load_hosted_identity_status() -> anyhow::Result<AgentHostedIdentityStatus> {
        Ok(Self::load_hosted_identity_snapshot()?.hosted_identity_status())
    }

    pub(crate) fn load_registered_agent(
        owner_id: &str,
        operation: &str,
    ) -> Result<AgentRegisteredAgent, AgentRegisteredAgentLoadError> {
        let registry = Self::load_registered_agent_registry_projection().map_err(|error| {
            let AgentRegistryProjectionLoadError::RegistryUnreadable { source } = error;
            AgentRegisteredAgentLoadError::RegistryUnreadable { source }
        })?;
        AgentRegisteredAgent::from_registry(&registry, owner_id, operation).map_err(Into::into)
    }

    pub(crate) fn load_registered_agent_registry_projection(
    ) -> Result<AgentRegistry, AgentRegistryProjectionLoadError> {
        agent_registry::load_agents()
            .map_err(|source| AgentRegistryProjectionLoadError::RegistryUnreadable { source })
    }

    pub(crate) fn load_registered_agent_workspace(
        owner_id: &str,
        operation: &str,
    ) -> Result<AgentRegisteredWorkspace, AgentRegisteredAgentLoadError> {
        Self::load_registered_agent(owner_id, operation).map(AgentRegisteredAgent::into_workspace)
    }

    pub(crate) fn try_load_snapshot(
    ) -> Result<AgentAggregateSnapshot, AgentAggregateSnapshotLoadError> {
        let registry = Self::load_registry_projection()?;
        let local_agents = Self::load_hosted_identity_projection()?;
        Ok(AgentAggregateSnapshot::new(registry, local_agents))
    }

    fn load_registry_projection() -> Result<AgentRegistry, AgentAggregateSnapshotLoadError> {
        Self::load_registered_agent_registry_projection().map_err(|error| {
            let AgentRegistryProjectionLoadError::RegistryUnreadable { source } = error;
            AgentAggregateSnapshotLoadError::RegistryUnreadable { source }
        })
    }

    fn load_hosted_identity_projection() -> Result<LocalAgentsFile, AgentAggregateSnapshotLoadError>
    {
        local_agents::load_for_fresh_host_projection()
            .map_err(|source| AgentAggregateSnapshotLoadError::IdentityUnreadable { source })
    }
}

#[cfg(test)]
mod tests {
    use super::agent_registry::{self, AgentEntry};
    use super::*;
    use crate::core::agent::spec::RuntimeKind;

    fn hosted_agent(profile: &str, name: &str, agent_ura: &str) -> HostedAgentEntry {
        HostedAgentEntry {
            profile: profile.to_string(),
            name: name.to_string(),
            agent_ura: agent_ura.to_string(),
            signing_authority: "hosted_by:easynet:///r/acme/device/dev-1".to_string(),
            first_seen_at: "2026-07-16T00:00:00Z".to_string(),
        }
    }

    fn snapshot(entries: Vec<HostedAgentEntry>) -> AgentAggregateSnapshot {
        AgentAggregateSnapshot::new(
            AgentRegistry::default(),
            LocalAgentsFile {
                host_device_ura: "easynet:///r/acme/device/dev-1".to_string(),
                hosted_agents: entries,
            },
        )
    }

    #[test]
    fn registered_agent_lookup_canonicalizes_surface_name() {
        let mut registry = AgentRegistry::default();
        registry.agents.insert(
            "default/claude".to_string(),
            AgentEntry::new(RuntimeKind::ClaudeCode, None),
        );
        let snapshot = AgentAggregateSnapshot::new(registry, LocalAgentsFile::default());

        assert!(snapshot.has_registered_agent("claude"));
        assert!(snapshot.has_registered_agent("default/claude"));
        assert!(!snapshot.has_registered_agent("Claude"));
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
            "default/claude".to_string(),
            AgentEntry::new(RuntimeKind::ClaudeCode, None),
        );
        registry.agents.insert(
            "research/codex".to_string(),
            AgentEntry::new(RuntimeKind::Codex, None),
        );
        let snapshot = AgentAggregateSnapshot::new(registry, LocalAgentsFile::default());

        let names = snapshot
            .registered_agent_surface_names()
            .expect("valid registered Agent surface projection");

        assert!(names.contains("claude"));
        assert!(names.contains("default/claude"));
        assert!(names.contains("codex"));
        assert!(names.contains("research/codex"));
    }

    #[test]
    fn registered_agent_names_reject_malformed_registry_keys() {
        let mut registry = AgentRegistry::default();
        registry.agents.insert(
            "../alice".to_string(),
            AgentEntry::new(RuntimeKind::ClaudeCode, None),
        );
        let snapshot = AgentAggregateSnapshot::new(registry, LocalAgentsFile::default());

        let error = snapshot
            .registered_agent_names()
            .expect_err("malformed registry key must fail closed");

        assert!(matches!(
            error,
            AgentRegisteredIdentityProjectionError::InvalidRegisteredAgentKey { .. }
        ));
        assert!(
            error.to_string().contains("not canonical"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn registered_agent_workspace_projects_root_and_skill_layout() {
        let root = std::env::temp_dir().join("easynet-agent-owner");
        let mut entry = AgentEntry::new(RuntimeKind::ClaudeCode, None);
        entry.root_path = Some(root.clone());
        let mut registry = AgentRegistry::default();
        registry.agents.insert("default/claude".to_string(), entry);
        let snapshot = AgentAggregateSnapshot::new(registry, LocalAgentsFile::default());

        let owner = snapshot
            .registered_agent_workspace("claude", "skill.publish")
            .expect("registered owner");

        assert_eq!(owner.root_path(), root.as_path());
        assert_eq!(owner.skill_layout(), AgentSkillLayout::ClaudeCode);

        let codex_root = std::env::temp_dir().join("easynet-codex-agent-owner");
        let mut codex_entry = AgentEntry::new(RuntimeKind::CodexAppServer, None);
        codex_entry.root_path = Some(codex_root);
        let mut registry = AgentRegistry::default();
        registry
            .agents
            .insert("default/codex".to_string(), codex_entry);
        let snapshot = AgentAggregateSnapshot::new(registry, LocalAgentsFile::default());
        let codex = snapshot
            .registered_agent_workspace("codex", "skill.publish")
            .expect("codex app server owner");
        assert_eq!(codex.skill_layout(), AgentSkillLayout::Codex);
    }

    #[test]
    fn registered_agent_runtime_projection_preserves_optional_forget_semantics() {
        let root = std::env::temp_dir().join("easynet-runtime-agent");
        let mut entry = AgentEntry::new(RuntimeKind::Codex, None);
        entry.root_path = Some(root.clone());
        let mut registry = AgentRegistry::default();
        registry
            .agents
            .insert("default/codex".to_string(), entry.clone());
        let snapshot = AgentAggregateSnapshot::new(registry, LocalAgentsFile::default());

        let projection = snapshot
            .registered_agent_runtime_projection("codex")
            .expect("registered runtime projection");

        assert_eq!(projection.entry().agent_type, entry.agent_type);
        assert_eq!(
            projection.ability_manifest_path("quote"),
            Some(root.join("abilities/quote.ability.toml"))
        );
        assert!(snapshot
            .registered_agent_runtime_projection("missing")
            .is_none());
    }

    #[test]
    fn registered_agent_workspace_reports_missing_and_corrupt_rows() {
        let mut registry = AgentRegistry::default();
        registry.agents.insert(
            "default/codex".to_string(),
            AgentEntry::new(RuntimeKind::Codex, None),
        );
        let snapshot = AgentAggregateSnapshot::new(registry, LocalAgentsFile::default());

        let missing = snapshot
            .registered_agent_workspace("claude", "skill.publish")
            .expect_err("missing owner");
        assert!(missing
            .to_string()
            .contains("registered agents: [\"default/codex\"]"));

        let corrupt = snapshot
            .registered_agent_workspace("codex", "skill.publish")
            .expect_err("missing root path");
        assert!(corrupt.to_string().contains("skill.publish"));
    }

    #[test]
    fn registry_only_workspace_lookup_ignores_unreadable_hosted_identity_state() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let root = crate::daemon::persistence::config::agents_root().join("claude");
        std::fs::create_dir_all(&root).expect("create registered workspace");
        let mut entry = AgentEntry::new(RuntimeKind::ClaudeCode, None);
        entry.root_path = Some(root.clone());
        let mut registry = AgentRegistry::default();
        registry.agents.insert("default/claude".to_string(), entry);
        agent_registry::save_agents(&registry).expect("save registered Agent");
        std::fs::write(local_agents::path(), b"{").expect("corrupt hosted identity projection");

        let workspace =
            AgentAggregateRepository::load_registered_agent_workspace("claude", "skill.install")
                .expect("registry-only workspace lookup");

        assert_eq!(workspace.root_path(), root.as_path());
        let registry = AgentAggregateRepository::load_registered_agent_registry_projection()
            .expect("registry-only Agent projection");
        assert!(registry.agents.contains_key("default/claude"));
        assert!(matches!(
            AgentAggregateRepository::try_load_snapshot(),
            Err(AgentAggregateSnapshotLoadError::IdentityUnreadable { .. })
        ));
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
    fn hosted_agent_identity_by_name_projects_authority_fields() {
        let snapshot = snapshot(vec![hosted_agent(
            "llm",
            "claude",
            "easynet:///r/acme/agent/u1.claude",
        )]);

        let identity = snapshot
            .hosted_agent_identity_by_name("claude")
            .unwrap()
            .expect("hosted identity");

        assert_eq!(identity.profile, "llm");
        assert_eq!(identity.name, "claude");
        assert_eq!(identity.agent_ura, "easynet:///r/acme/agent/u1.claude");
        assert_eq!(
            identity.signing_authority,
            "hosted_by:easynet:///r/acme/device/dev-1"
        );
    }

    #[test]
    fn hosted_agent_identity_by_ura_validates_local_hosted_membership() {
        let snapshot = snapshot(vec![hosted_agent(
            "llm",
            "claude",
            "easynet:///r/acme/agent/u1.claude",
        )]);

        assert_eq!(
            snapshot
                .hosted_agent_identity_by_ura("easynet:///r/acme/agent/u1.claude")
                .expect("hosted identity")
                .name,
            "claude"
        );
        assert!(snapshot
            .hosted_agent_identity_by_ura("easynet:///r/acme/agent/u1.other")
            .is_none());
    }

    #[test]
    fn hosted_identity_status_projects_joined_state_and_count() {
        let snapshot = AgentAggregateSnapshot::new(
            AgentRegistry::default(),
            LocalAgentsFile {
                host_device_ura: "  easynet:///r/acme/device/dev-1  ".to_string(),
                hosted_agents: vec![
                    hosted_agent("llm", "claude", "easynet:///r/acme/agent/u1.claude"),
                    hosted_agent("mcp", "server", "easynet:///r/acme/agent/u1.server"),
                ],
            },
        );

        let status = snapshot.hosted_identity_status();

        assert!(status.is_joined());
        assert_eq!(
            status.host_device_ura(),
            Some("easynet:///r/acme/device/dev-1")
        );
        assert_eq!(status.hosted_agent_count(), 2);
    }

    #[test]
    fn hosted_identity_status_treats_blank_host_as_unjoined() {
        let snapshot = AgentAggregateSnapshot::new(
            AgentRegistry::default(),
            LocalAgentsFile {
                host_device_ura: "   ".to_string(),
                hosted_agents: vec![hosted_agent(
                    "llm",
                    "claude",
                    "easynet:///r/acme/agent/u1.claude",
                )],
            },
        );

        let status = snapshot.hosted_identity_status();

        assert!(!status.is_joined());
        assert_eq!(status.host_device_ura(), None);
        assert_eq!(status.hosted_agent_count(), 1);
    }

    #[test]
    fn hosted_identity_snapshot_resolves_llm_owner_ura_without_registry() {
        let snapshot = AgentHostedIdentitySnapshot::new(LocalAgentsFile {
            host_device_ura: "easynet:///r/acme/device/dev-1".to_string(),
            hosted_agents: vec![
                hosted_agent("llm", "claude", "easynet:///r/acme/agent/u1.claude"),
                hosted_agent("mcp", "claude", "easynet:///r/acme/agent/u1.mcp"),
            ],
        });

        assert_eq!(
            snapshot.hosted_llm_agent_ura("claude"),
            Some("easynet:///r/acme/agent/u1.claude")
        );
        assert_eq!(snapshot.hosted_llm_agent_ura("missing"), None);
    }

    #[test]
    fn hosted_skill_owner_projection_resolves_names_and_uras() {
        let snapshot = snapshot(vec![
            hosted_agent("llm", "claude", "easynet:///r/acme/agent/u1.claude"),
            hosted_agent("mcp", "tools", "easynet:///r/acme/agent/u1.tools"),
            hosted_agent("llm", "alias", "easynet:///r/acme/agent/u1.claude"),
        ]);

        let projection = snapshot.hosted_skill_owner_projection();

        assert_eq!(
            projection.hosted_ura_for("claude"),
            Some("easynet:///r/acme/agent/u1.claude")
        );
        assert_eq!(
            projection.hosted_ura_for("tools"),
            Some("easynet:///r/acme/agent/u1.tools")
        );
        assert_eq!(
            projection.owner_name_for_agent_ura("easynet:///r/acme/agent/u1.claude"),
            Some("claude")
        );
        assert_eq!(
            projection.owner_name_for_agent_ura("easynet:///r/acme/agent/u1.missing"),
            None
        );
    }

    #[test]
    fn hosted_identity_snapshot_rejects_ambiguous_llm_owner_ura() {
        let snapshot = AgentHostedIdentitySnapshot::new(LocalAgentsFile {
            host_device_ura: "easynet:///r/acme/device/dev-1".to_string(),
            hosted_agents: vec![
                hosted_agent("llm", "same", "easynet:///r/acme/agent/u1.same"),
                hosted_agent("llm", "same", "easynet:///r/acme/agent/u1.same2"),
            ],
        });

        assert_eq!(snapshot.hosted_llm_agent_ura("same"), None);
    }

    #[test]
    fn hosted_agent_authority_roots_preserve_hosted_identity_order() {
        let snapshot = AgentHostedIdentitySnapshot::new(LocalAgentsFile {
            host_device_ura: "easynet:///r/acme/device/dev-1".to_string(),
            hosted_agents: vec![
                hosted_agent("llm", "claude", "easynet:///r/acme/agent/u1.claude"),
                hosted_agent("mcp", "default", "easynet:///r/acme/agent/u1.mcp"),
            ],
        });

        assert_eq!(
            snapshot.hosted_agent_authority_roots().unwrap(),
            vec![
                "easynet:///r/acme/agent/u1.claude".to_string(),
                "easynet:///r/acme/agent/u1.mcp".to_string(),
            ]
        );
    }

    #[test]
    fn hosted_agent_authority_roots_reject_raw_polluted_identity_rows() {
        let snapshot = AgentHostedIdentitySnapshot::new(LocalAgentsFile {
            host_device_ura: "easynet:///r/acme/device/dev-1".to_string(),
            hosted_agents: vec![HostedAgentEntry {
                profile: "llm".to_string(),
                name: "claude".to_string(),
                agent_ura: "easynet:///r/other/agent/u1.claude".to_string(),
                signing_authority: "hosted_by:easynet:///r/acme/device/dev-1".to_string(),
                first_seen_at: "2026-07-16T00:00:00Z".to_string(),
            }],
        });

        let error = snapshot
            .hosted_agent_authority_roots()
            .expect_err("authority roots must consume validated aggregate rows");

        assert!(
            error.to_string().contains("does not match host realm"),
            "{error}"
        );
    }

    #[test]
    fn hosted_advertise_entries_project_only_persisted_configured_agents() {
        let snapshot = AgentHostedIdentitySnapshot::new(LocalAgentsFile {
            host_device_ura: "easynet:///r/acme/device/dev-1".to_string(),
            hosted_agents: vec![
                hosted_agent("llm", "claude", "easynet:///r/acme/agent/u1.claude"),
                hosted_agent("llm", "duplicate", "easynet:///r/acme/agent/u1.claude"),
                hosted_agent("llm", "other-user", "easynet:///r/acme/agent/u2.other"),
                hosted_agent("llm", "other-realm", "easynet:///r/other/agent/u1.other"),
                hosted_agent(
                    "mcp",
                    "device-sponsored",
                    "easynet:///r/acme/agent/device.dev-1.mcp-default",
                ),
                hosted_agent("llm", "device", "easynet:///r/acme/device/dev-2"),
                hosted_agent("llm", "pending", "<unjoined>"),
                hosted_agent("mcp", "blank", "   "),
            ],
        });

        let entries = snapshot.hosted_advertise_entries("acme", "u1");
        let rows = entries
            .iter()
            .map(|entry| (entry.agent_ura(), entry.short_label()))
            .collect::<Vec<_>>();

        assert_eq!(
            rows,
            vec![("easynet:///r/acme/agent/u1.claude", "u1.claude")]
        );
    }

    #[test]
    fn hosted_advertise_entries_require_exact_runtime_user_context() {
        let snapshot = AgentHostedIdentitySnapshot::new(LocalAgentsFile {
            host_device_ura: "easynet:///r/acme/device/dev-1".to_string(),
            hosted_agents: vec![hosted_agent(
                "llm",
                "claude",
                "easynet:///r/acme/agent/u1.claude",
            )],
        });

        assert!(snapshot.hosted_advertise_entries("acme", "self").is_empty());
        assert!(snapshot.hosted_advertise_entries("", "u1").is_empty());
        assert!(snapshot.hosted_advertise_entries("acme", "").is_empty());
    }

    #[test]
    fn hosted_identity_snapshot_projects_host_descriptor_owners() {
        let snapshot = AgentHostedIdentitySnapshot::new(LocalAgentsFile {
            host_device_ura: "  easynet:///r/acme/device/dev-1  ".to_string(),
            hosted_agents: vec![
                hosted_agent("mcp", "default", "easynet:///r/acme/agent/u1.mcp"),
                hosted_agent("llm", "claude", "easynet:///r/acme/agent/u1.claude"),
                hosted_agent("llm", "codex", "easynet:///r/acme/agent/u1.codex"),
            ],
        });

        let projection = snapshot.host_descriptor_identity_projection();

        assert_eq!(
            projection.host_device_ura(),
            Some("easynet:///r/acme/device/dev-1")
        );
        assert_eq!(
            projection.llm_agent_uras(),
            &[
                (
                    "claude".to_string(),
                    "easynet:///r/acme/agent/u1.claude".to_string()
                ),
                (
                    "codex".to_string(),
                    "easynet:///r/acme/agent/u1.codex".to_string()
                ),
            ]
        );
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
            "default/claude".to_string(),
            AgentEntry::new(RuntimeKind::ClaudeCode, None),
        );
        registry.agents.insert(
            "default/codex".to_string(),
            AgentEntry::new(RuntimeKind::Codex, None),
        );
        let snapshot = AgentAggregateSnapshot::new(
            registry,
            LocalAgentsFile {
                host_device_ura: "easynet:///r/acme/agent/device".to_string(),
                hosted_agents: vec![hosted_agent(
                    "llm",
                    "claude",
                    "easynet:///r/acme/agent/u1.claude",
                )],
            },
        );

        let projection = snapshot
            .local_target_projection()
            .expect("valid local target projection");

        assert!(projection
            .hosted_agent_targets
            .contains(&HostedAgentTarget {
                realm: "acme".to_string(),
                user_id: "u1".to_string(),
                agent_id: "claude".to_string(),
            }));
        assert_eq!(projection.hosted_agent_targets.len(), 1);
        assert!(projection.registered_agent_ids.contains("claude"));
        assert!(projection.registered_agent_ids.contains("codex"));
    }

    #[test]
    fn local_target_projection_rejects_malformed_hosted_identities() {
        for (case, entry, expected) in [
            (
                "malformed hosted Agent URA",
                hosted_agent("llm", "malformed", "not-a-ura"),
                "invalid Agent URA",
            ),
            (
                "non-Agent hosted identity",
                hosted_agent("llm", "bad", "easynet:///r/acme/device/dev-1"),
                "non-Agent URA",
            ),
        ] {
            let snapshot = AgentAggregateSnapshot::new(
                AgentRegistry::default(),
                LocalAgentsFile {
                    host_device_ura: "easynet:///r/acme/agent/device".to_string(),
                    hosted_agents: vec![entry],
                },
            );

            let error = snapshot.local_target_projection().expect_err(case);

            assert!(
                error.to_string().contains(expected),
                "{case} should report {expected:?}, got: {error}"
            );
        }
    }

    #[test]
    fn local_target_projection_rejects_malformed_registered_agent_keys() {
        let mut registry = AgentRegistry::default();
        registry.agents.insert(
            "../alice".to_string(),
            AgentEntry::new(RuntimeKind::ClaudeCode, None),
        );
        let snapshot = AgentAggregateSnapshot::new(
            registry,
            LocalAgentsFile {
                host_device_ura: "easynet:///r/acme/agent/device".to_string(),
                hosted_agents: vec![hosted_agent(
                    "llm",
                    "claude",
                    "easynet:///r/acme/agent/u1.claude",
                )],
            },
        );

        let error = snapshot
            .local_target_projection()
            .expect_err("malformed registry key must fail closed");

        assert!(
            error.to_string().contains("registered Agent registry key"),
            "unexpected error: {error}"
        );
        assert!(
            error.to_string().contains("not canonical"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn hosted_agent_placements_project_valid_agent_hosts() {
        let snapshot = AgentAggregateSnapshot::new(
            AgentRegistry::default(),
            LocalAgentsFile {
                host_device_ura: "easynet:///r/acme/device/dev-1".to_string(),
                hosted_agents: vec![hosted_agent(
                    "llm",
                    "claude",
                    "easynet:///r/acme/agent/u1.claude",
                )],
            },
        );

        let projection = snapshot
            .hosted_agent_placements()
            .expect("valid hosted Agent placement projection");

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
                host_device_ura: String::new(),
                hosted_agents: vec![hosted_agent(
                    "llm",
                    "claude",
                    "easynet:///r/acme/agent/u1.claude",
                )],
            },
        )
        .hosted_agent_placements();

        let projection = projection.expect("empty host device is first-boot empty placement");
        assert!(projection.by_agent_ura.is_empty());
    }

    #[test]
    fn hosted_agent_placements_reject_malformed_hosted_identities() {
        for (case, entry, expected) in [
            (
                "malformed hosted Agent URA",
                hosted_agent("llm", "bad", "not-a-ura"),
                "invalid Agent URA",
            ),
            (
                "non-Agent hosted identity",
                hosted_agent("mcp", "default", "easynet:///r/acme/device/dev-1"),
                "non-Agent URA",
            ),
        ] {
            let error = AgentAggregateSnapshot::new(
                AgentRegistry::default(),
                LocalAgentsFile {
                    host_device_ura: "easynet:///r/acme/device/dev-1".to_string(),
                    hosted_agents: vec![entry],
                },
            )
            .hosted_agent_placements()
            .expect_err(case);

            assert!(
                error.to_string().contains(expected),
                "{case} should report {expected:?}, got: {error}"
            );
        }
    }
}
