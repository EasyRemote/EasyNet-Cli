//! File: `src/daemon/ability/builtins/agents/lifecycle.rs`
//! Description: Transactional `agent.{start,stop,refresh}` handlers.
//!
//! Protocol responsibility: atomically converge the durable agent registry,
//! hosted identity index, authority inventory, ability catalog, and live Axon
//! runtime. A successful response means every local segment committed; Hub
//! advertisement remains best-effort but is always represented explicitly.
//!
//! Implementation approach: lifecycle mutations advance through an explicit
//! state machine and retain pre-mutation snapshots. Purge isolates the
//! registered root with a same-directory atomic rename before durable commit,
//! then deletes only that quarantine after commit. Any local failure triggers
//! reverse-order compensation; incomplete compensation is returned as a typed
//! partial failure. Registrar readiness is a hard precondition, never a
//! boot-window no-op or restart repair path.
//
// EasyNet CLI — agent.{start,stop} ability handlers
// =================================================================
//
// Per RFC §18, the device-profile advertises these abilities under
// device authority. They match the operator-facing `easynet agent add` /
// `easynet agent remove`
// CLI subcommands but reach the same registry through Invoke instead
// of stdin parsing — so a remote operator (or another local Agent)
// can manage the local agent registry without spawning a shell.
//
// Lifecycle model
// ---------------
// LLM sub-agents in EasyNet are *registry rows*, not resident
// processes. Per `~/.easynet/agents.json`, an entry records the
// runtime kind (claude-code / codex / …), the model selector, and
// optional label. The actual claude/codex process is spawned per
// invocation by `agent send` / chat_ability and exits when the
// invocation completes — there is no long-running daemon to start
// or stop.
//
// So `agent.start` is "register a new agent row", and
// `agent.stop` is "remove the row." The ability names match
// the §18 registry; the verbs map onto today's reality.
//
// What lives here
// ---------------
//   * agent.start — { name, agent_type, model? } →
//                          { agent_ura, replaced_prior, runtime_* }
//                          replaced_prior=true means the call
//                          overwrote an existing row of the same
//                          name (operator-visible event).
//   * agent.stop  — { name | agent_ura, purge? } → { ack: bool }
//                          ack=false when the row didn't exist
//                          (idempotent: callers can retry without
//                          triggering an error).
//
// What does NOT live here
// -----------------------
//   * Process kill signals. There are no resident agent processes
//     today (see Lifecycle model above); a future per-agent
//     long-runner would land its own a future device.session operation.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::core::agent::id::AgentId;
use crate::core::agent::spec::{AgentSpec, RuntimeKind};
use crate::daemon::ability::catalog::profiles::bootstrap::{self, BootstrapPlan, UuidMinter};
use crate::daemon::ability::dispatch::AxonAbilityCatalog;
use crate::daemon::axon_bridge::hot_agent_registrar::{
    block_on_hot_registrar, HotAgentAdvertiseRequest, HotAgentAdvertiseState, HotAgentAdvertiser,
    HotAgentProjectionRequest, HotAgentRegistrar, HotAgentRegistrarError, HotAgentRevokeRequest,
};
use crate::daemon::execution::mission::directory::{AgentDirectory, Location};
use crate::daemon::persistence::agent_lifecycle::{
    self as lifecycle_store, AgentLifecycleMutationGuard, AgentPurgeJournal,
    AgentPurgeNoPublicationReason, AgentPurgePublication, AgentPurgePublicationEntry,
    AgentPurgePublicationPlan, AgentPurgePublicationRetry, AgentPurgePublicationStage,
    AgentPurgeStage, AgentRootIdentity,
};
use crate::daemon::persistence::agent_registry as agents;
use crate::daemon::persistence::agent_registry::{
    AgentEntry, AgentRegistry, AgentType, CURRENT_REGISTRY_SCHEMA,
};
use crate::daemon::persistence::{config, local_agents};

use crate::daemon::ability::dispatch::OwnerKind;
pub const ABILITY_START_AGENT: &str = crate::daemon::ability::names::agents::AGENT_START;
pub const ABILITY_STOP_AGENT: &str = crate::daemon::ability::names::agents::AGENT_STOP;
pub const ABILITY_PURGE_AGENT: &str = crate::daemon::ability::names::agents::AGENT_PURGE;
pub const ABILITY_RECONCILE_AGENT_PURGE: &str =
    crate::daemon::ability::names::agents::AGENT_PURGE_RECONCILE;
pub const ABILITY_REFRESH_AGENTS: &str = crate::daemon::ability::names::agents::AGENT_REFRESH;

/// Single-assignment registrar shared by catalogue assembly and lifecycle
/// handler closures. The cell breaks the construction cycle; it is not a
/// degradation seam. Missing or pending state is a typed hard error before any
/// lifecycle mutation touches disk.
pub type SharedHotRegistrarCell =
    std::sync::OnceLock<Arc<crate::daemon::axon_bridge::hot_agent_registrar::HotAgentRegistrar>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentLifecycleState {
    Prepared,
    Materialized,
    DurablePersisted,
    IdentityPersisted,
    RuntimeSynchronized,
    AuthoritySynchronized,
    Committed,
    RollingBack,
    RolledBack,
    PartialFailure,
}

impl std::fmt::Display for AgentLifecycleState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Prepared => "prepared",
            Self::Materialized => "materialized",
            Self::DurablePersisted => "durable_persisted",
            Self::IdentityPersisted => "identity_persisted",
            Self::RuntimeSynchronized => "runtime_synchronized",
            Self::AuthoritySynchronized => "authority_synchronized",
            Self::Committed => "committed",
            Self::RollingBack => "rolling_back",
            Self::RolledBack => "rolled_back",
            Self::PartialFailure => "partial_failure",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentLifecyclePlan {
    Start,
    Stop,
}

#[derive(Debug, thiserror::Error)]
enum AgentLifecycleError {
    #[error("{operation}: hot-Agent registrar is not wired")]
    RegistrarUnavailable { operation: &'static str },
    #[error("{operation}: registrar precondition failed: {source}")]
    Registrar {
        operation: &'static str,
        #[source]
        source: Box<HotAgentRegistrarError>,
    },
    #[error("{operation} failed in lifecycle state {state}: {cause}; rollback={rollback}")]
    Mutation {
        operation: &'static str,
        state: AgentLifecycleState,
        cause: String,
        rollback: String,
    },
}

#[derive(Debug, Default)]
enum MaterializationRollback {
    #[default]
    None,
    Created {
        root: std::path::PathBuf,
        root_preexisted: bool,
    },
    UpdatedSpec {
        path: std::path::PathBuf,
        prior_bytes: Vec<u8>,
    },
}

impl MaterializationRollback {
    fn rollback(&self) -> anyhow::Result<()> {
        match self {
            Self::None => Ok(()),
            Self::UpdatedSpec { path, prior_bytes } => config::atomic_write(path, prior_bytes)
                .map_err(|error| anyhow::anyhow!("restore {}: {error}", path.display())),
            Self::Created {
                root,
                root_preexisted,
            } => {
                if !root.exists() {
                    return Ok(());
                }
                if *root_preexisted {
                    for relative in [
                        "agent.toml",
                        ".env",
                        "abilities",
                        "skills",
                        "memory",
                        "runs",
                    ] {
                        let path = root.join(relative);
                        match std::fs::remove_dir_all(&path) {
                            Ok(()) => {}
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                            Err(error) if path.is_file() => {
                                std::fs::remove_file(&path).map_err(|remove_error| {
                                    anyhow::anyhow!(
                                        "remove rollback artifact {}: {error}; file removal: {remove_error}",
                                        path.display()
                                    )
                                })?;
                            }
                            Err(error) => {
                                return Err(anyhow::anyhow!(
                                    "remove rollback artifact {}: {error}",
                                    path.display()
                                ));
                            }
                        }
                    }
                    Ok(())
                } else {
                    std::fs::remove_dir_all(root).map_err(|error| {
                        anyhow::anyhow!("remove created agent root {}: {error}", root.display())
                    })
                }
            }
        }
    }
}

struct OpenRegisteredAgentRoot {
    root: std::path::PathBuf,
    handle: Option<std::fs::File>,
    identity: AgentRootIdentity,
}

enum RegisteredAgentRoot {
    Present(OpenRegisteredAgentRoot),
    Absent(std::path::PathBuf),
}

fn validate_registered_agent_root(
    name: &str,
    registered_root: &std::path::Path,
) -> anyhow::Result<RegisteredAgentRoot> {
    use std::path::Component;

    if !registered_root.is_absolute() {
        anyhow::bail!(
            "agent.purge: registered root for `{name}` is not absolute: {}; refusing purge",
            registered_root.display()
        );
    }
    if registered_root
        .components()
        .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        anyhow::bail!(
            "agent.purge: registered root for `{name}` is not normalized: {}; refusing purge",
            registered_root.display()
        );
    }
    let metadata = match std::fs::symlink_metadata(registered_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = registered_root.parent().ok_or_else(|| {
                anyhow::anyhow!("agent.purge: registered root has no parent; refusing purge")
            })?;
            let canonical_parent = std::fs::canonicalize(parent).map_err(|parent_error| {
                anyhow::anyhow!(
                    "agent.purge: cannot validate absent registered root {} because parent {} is not canonical: {parent_error}",
                    registered_root.display(),
                    parent.display()
                )
            })?;
            #[cfg(unix)]
            if canonical_parent != parent {
                anyhow::bail!(
                    "agent.purge: absent registered root {} has non-canonical parent {} (canonical {}); refusing purge",
                    registered_root.display(),
                    parent.display(),
                    canonical_parent.display()
                );
            }
            return Ok(RegisteredAgentRoot::Absent(registered_root.to_path_buf()));
        }
        Err(error) => {
            return Err(anyhow::anyhow!(
                "agent.purge: inspect registered root {}: {error}",
                registered_root.display()
            ));
        }
    };
    if metadata.file_type().is_symlink() {
        anyhow::bail!(
            "agent.purge: registered root {} is a symlink; refusing purge",
            registered_root.display()
        );
    }
    if !metadata.is_dir() {
        anyhow::bail!(
            "agent.purge: registered root {} is not a directory; refusing purge",
            registered_root.display()
        );
    }
    let canonical_root = std::fs::canonicalize(registered_root).map_err(|error| {
        anyhow::anyhow!(
            "agent.purge: canonicalize registered root {}: {error}",
            registered_root.display()
        )
    })?;
    #[cfg(unix)]
    if canonical_root != registered_root {
        anyhow::bail!(
            "agent.purge: registered root {} is not canonical (canonical {}); refusing purge",
            registered_root.display(),
            canonical_root.display()
        );
    }
    #[cfg(unix)]
    let validated_root = canonical_root;
    #[cfg(not(unix))]
    let validated_root = registered_root.to_path_buf();
    let directory = AgentDirectory::open(&validated_root).map_err(|error| {
        anyhow::anyhow!(
            "agent.purge: registered root {} is not a valid agent directory: {error:#}",
            validated_root.display()
        )
    })?;
    if directory.spec().name != name {
        anyhow::bail!(
            "agent.purge: registered root {} belongs to agent `{}`, not `{name}`; refusing purge",
            validated_root.display(),
            directory.spec().name
        );
    }
    #[cfg(unix)]
    let handle = Some(std::fs::File::open(&validated_root).map_err(|error| {
        anyhow::anyhow!(
            "agent.purge: open registered root handle {}: {error}",
            validated_root.display()
        )
    })?);
    #[cfg(not(unix))]
    let handle: Option<std::fs::File> = None;
    #[cfg(unix)]
    let identity_metadata = handle
        .as_ref()
        .map(std::fs::File::metadata)
        .transpose()?
        .unwrap_or(metadata);
    #[cfg(unix)]
    let identity = AgentRootIdentity::from_metadata(&identity_metadata).map_err(|error| {
        anyhow::anyhow!(
            "agent.purge: derive filesystem identity for {}: {error:#}",
            validated_root.display()
        )
    })?;
    #[cfg(not(unix))]
    let identity = AgentRootIdentity::from_path(&validated_root).map_err(|error| {
        anyhow::anyhow!(
            "agent.purge: derive filesystem identity for {}: {error:#}",
            validated_root.display()
        )
    })?;
    Ok(RegisteredAgentRoot::Present(OpenRegisteredAgentRoot {
        root: validated_root,
        handle,
        identity,
    }))
}

#[derive(Debug, Default, Clone, Copy)]
struct AgentLifecycleProjectionStore;

impl AgentLifecycleProjectionStore {
    fn persist_registry(&self, registry: &AgentRegistry) -> anyhow::Result<()> {
        agents::save_agents(registry)
            .map_err(|error| anyhow::anyhow!("persist durable agent registry: {error:#}"))
    }

    fn persist_identities(&self, identities: &local_agents::LocalAgentsFile) -> anyhow::Result<()> {
        local_agents::save(identities)
            .map_err(|error| anyhow::anyhow!("persist hosted-Agent identity registry: {error:#}"))
    }

    fn restore_registry_snapshot(&self, registry: &AgentRegistry) -> anyhow::Result<()> {
        agents::save_agents(registry).map_err(Into::into)
    }

    fn restore_identity_snapshot(
        &self,
        identities: &local_agents::LocalAgentsFile,
    ) -> anyhow::Result<()> {
        local_agents::save(identities).map_err(Into::into)
    }

    fn restore_uncommitted_purge_snapshots(
        &self,
        journal: &AgentPurgeJournal,
    ) -> anyhow::Result<()> {
        self.restore_registry_snapshot(&journal.original_registry)
            .map_err(|error| anyhow::anyhow!("recover Agent purge agents.json: {error:#}"))?;
        self.restore_identity_snapshot(&journal.original_local_agents)
            .map_err(|error| anyhow::anyhow!("recover Agent purge local-agents.json: {error:#}"))
    }
}

struct AgentLifecycleTransaction {
    operation: &'static str,
    plan: AgentLifecyclePlan,
    state: AgentLifecycleState,
    projections: AgentLifecycleProjectionStore,
    original_registry: AgentRegistry,
    original_local_agents: local_agents::LocalAgentsFile,
    registry_written: bool,
    identity_written: bool,
    materialization: MaterializationRollback,
}

impl AgentLifecycleTransaction {
    fn for_start(
        operation: &'static str,
        original_registry: AgentRegistry,
        original_local_agents: local_agents::LocalAgentsFile,
    ) -> Self {
        Self {
            operation,
            plan: AgentLifecyclePlan::Start,
            state: AgentLifecycleState::Prepared,
            projections: AgentLifecycleProjectionStore,
            original_registry,
            original_local_agents,
            registry_written: false,
            identity_written: false,
            materialization: MaterializationRollback::None,
        }
    }

    fn for_stop(
        operation: &'static str,
        original_registry: AgentRegistry,
        original_local_agents: local_agents::LocalAgentsFile,
    ) -> Self {
        Self {
            operation,
            plan: AgentLifecyclePlan::Stop,
            state: AgentLifecycleState::Prepared,
            projections: AgentLifecycleProjectionStore,
            original_registry,
            original_local_agents,
            registry_written: false,
            identity_written: false,
            materialization: MaterializationRollback::None,
        }
    }

    fn transition(&mut self, next: AgentLifecycleState) -> anyhow::Result<()> {
        let valid = match self.plan {
            AgentLifecyclePlan::Start => matches!(
                (self.state, next),
                (
                    AgentLifecycleState::Prepared,
                    AgentLifecycleState::Materialized
                ) | (
                    AgentLifecycleState::Prepared,
                    AgentLifecycleState::DurablePersisted
                ) | (
                    AgentLifecycleState::Materialized,
                    AgentLifecycleState::DurablePersisted
                ) | (
                    AgentLifecycleState::DurablePersisted,
                    AgentLifecycleState::IdentityPersisted
                ) | (
                    AgentLifecycleState::IdentityPersisted,
                    AgentLifecycleState::RuntimeSynchronized
                ) | (
                    AgentLifecycleState::RuntimeSynchronized,
                    AgentLifecycleState::Committed
                )
            ),
            AgentLifecyclePlan::Stop => matches!(
                (self.state, next),
                (
                    AgentLifecycleState::Prepared,
                    AgentLifecycleState::RuntimeSynchronized
                ) | (
                    AgentLifecycleState::RuntimeSynchronized,
                    AgentLifecycleState::DurablePersisted
                ) | (
                    AgentLifecycleState::DurablePersisted,
                    AgentLifecycleState::IdentityPersisted
                ) | (
                    AgentLifecycleState::IdentityPersisted,
                    AgentLifecycleState::AuthoritySynchronized
                ) | (
                    AgentLifecycleState::AuthoritySynchronized,
                    AgentLifecycleState::Committed
                )
            ),
        };
        if !valid {
            anyhow::bail!(
                "{}: invalid {:?} lifecycle transition {} -> {}",
                self.operation,
                self.plan,
                self.state,
                next
            );
        }
        self.state = next;
        Ok(())
    }

    fn record_materialization(&mut self, rollback: MaterializationRollback) -> anyhow::Result<()> {
        self.transition(AgentLifecycleState::Materialized)?;
        self.materialization = rollback;
        Ok(())
    }

    fn persist(
        &mut self,
        registry: &AgentRegistry,
        identities: &local_agents::LocalAgentsFile,
    ) -> Result<(), AgentLifecycleError> {
        self.persist_registry_projection(registry)
            .map_err(|error| self.failure_with_rollback(error.to_string()))?;
        self.persist_identity_projection(identities)
            .map_err(|error| self.failure_with_rollback(error.to_string()))?;
        Ok(())
    }

    fn mark_runtime_synchronized(&mut self) -> anyhow::Result<()> {
        self.transition(AgentLifecycleState::RuntimeSynchronized)
    }

    fn mark_registry_write_started(&mut self) {
        self.registry_written = true;
    }

    fn persist_registry_projection(&mut self, registry: &AgentRegistry) -> anyhow::Result<()> {
        // Mark each segment before the atomic-write call: a post-rename
        // directory-sync error means the new bytes may already be visible and
        // therefore still require compensation.
        self.mark_registry_write_started();
        self.projections.persist_registry(registry)?;
        self.mark_registry_persisted()
    }

    fn mark_registry_persisted(&mut self) -> anyhow::Result<()> {
        self.transition(AgentLifecycleState::DurablePersisted)
    }

    fn mark_identity_write_started(&mut self) {
        self.identity_written = true;
    }

    fn persist_identity_projection(
        &mut self,
        identities: &local_agents::LocalAgentsFile,
    ) -> anyhow::Result<()> {
        self.mark_identity_write_started();
        self.projections.persist_identities(identities)?;
        self.mark_identity_persisted()
    }

    fn mark_identity_persisted(&mut self) -> anyhow::Result<()> {
        self.transition(AgentLifecycleState::IdentityPersisted)
    }

    fn mark_authority_synchronized(&mut self) -> anyhow::Result<()> {
        self.transition(AgentLifecycleState::AuthoritySynchronized)
    }

    fn commit(&mut self) -> anyhow::Result<()> {
        self.transition(AgentLifecycleState::Committed)
    }

    fn failure_with_rollback(&mut self, cause: String) -> AgentLifecycleError {
        let failed_state = self.state;
        let rollback_failures = self.rollback();
        AgentLifecycleError::Mutation {
            operation: self.operation,
            state: failed_state,
            cause,
            rollback: if rollback_failures.is_empty() {
                "completed".to_string()
            } else {
                format!("partial({})", rollback_failures.join("; "))
            },
        }
    }

    fn rollback(&mut self) -> Vec<String> {
        let mut failures = Vec::new();
        if matches!(
            self.state,
            AgentLifecycleState::Committed
                | AgentLifecycleState::RolledBack
                | AgentLifecycleState::PartialFailure
        ) {
            return vec![format!(
                "invalid rollback transition from terminal state {}",
                self.state
            )];
        }
        self.state = AgentLifecycleState::RollingBack;
        if self.identity_written {
            if let Err(error) = self
                .projections
                .restore_identity_snapshot(&self.original_local_agents)
            {
                failures.push(format!("restore local-agents.json: {error:#}"));
            }
        }
        if self.registry_written {
            if let Err(error) = self
                .projections
                .restore_registry_snapshot(&self.original_registry)
            {
                failures.push(format!("restore agents.json: {error:#}"));
            }
        }
        if let Err(error) = self.materialization.rollback() {
            failures.push(format!("restore agent directory: {error:#}"));
        }
        let terminal = if failures.is_empty() {
            AgentLifecycleState::RolledBack
        } else {
            AgentLifecycleState::PartialFailure
        };
        debug_assert_eq!(self.state, AgentLifecycleState::RollingBack);
        self.state = terminal;
        failures
    }
}

fn require_hot_registrar(
    hot_registrar: &SharedHotRegistrarCell,
    operation: &'static str,
) -> Result<Arc<HotAgentRegistrar>, AgentLifecycleError> {
    let registrar = hot_registrar
        .get()
        .cloned()
        .ok_or(AgentLifecycleError::RegistrarUnavailable { operation })?;
    registrar
        .require_ready()
        .map_err(|source| AgentLifecycleError::Registrar {
            operation,
            source: Box::new(source),
        })?;
    Ok(registrar)
}

pub fn register(reg: &mut AxonAbilityCatalog, hot_registrar: Arc<SharedHotRegistrarCell>) {
    super::authoring::register(reg, Arc::clone(&hot_registrar));
    let registrar_for_start = Arc::clone(&hot_registrar);
    reg.register_rpc_with_owner(
        "agent.start",
        OwnerKind::Device,
        Arc::new(move |args: Value| start_agent_handler(args, &registrar_for_start)),
    );
    let registrar_for_stop = Arc::clone(&hot_registrar);
    reg.register_rpc_with_owner(
        "agent.stop",
        OwnerKind::Device,
        Arc::new(move |args: Value| stop_agent_handler(args, &registrar_for_stop)),
    );
    let registrar_for_purge = Arc::clone(&hot_registrar);
    reg.register_rpc_with_owner(
        ABILITY_PURGE_AGENT,
        OwnerKind::Device,
        Arc::new(move |args: Value| purge_agent_handler(args, &registrar_for_purge)),
    );
    reg.register_rpc_with_envelope_and_owner(
        ABILITY_RECONCILE_AGENT_PURGE,
        OwnerKind::Device,
        Arc::new(purge_reconciliation_handler),
    );
    let registrar_for_refresh = Arc::clone(&hot_registrar);
    reg.register_rpc_with_owner(
        "agent.refresh",
        OwnerKind::Device,
        Arc::new(move |args: Value| refresh_agents_handler(args, &registrar_for_refresh)),
    );
}

fn purge_reconciliation_handler(
    envelope: crate::daemon::ability::dispatch::EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    let transaction_id = args
        .get("transaction_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("agent.purge.reconcile: transaction_id is required"))?;
    let command_id = args
        .get("command_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("agent.purge.reconcile: command_id is required"))?;
    if args
        .get("action")
        .and_then(Value::as_str)
        .is_some_and(|action| action != "retry")
    {
        anyhow::bail!("agent.purge.reconcile: only `retry` is supported");
    }
    let actor_ura = envelope.caller().trim();
    if actor_ura.is_empty() {
        anyhow::bail!("agent.purge.reconcile: admitted caller identity is missing");
    }
    let command = lifecycle_store::AgentPurgeReconciliationCommand {
        command_id: command_id.to_string(),
        transaction_id: transaction_id.to_string(),
        actor_ura: actor_ura.to_string(),
        action: lifecycle_store::AgentPurgePublicationReconciliation::Retry,
    };
    let authorization = lifecycle_store::AuthorizedPurgeReconciliation::from_admission(
        actor_ura,
        format!(
            "admission:{}:{}",
            envelope.invocation_id(),
            envelope.ability()
        ),
    )?;
    let outcome = lifecycle_store::reconcile_publication(
        &command,
        &authorization,
        purge_publication_now_unix_ms()?,
    )?;
    Ok(json!({
        "transaction_id": outcome.entry.transaction_id,
        "command_id": command.command_id,
        "action": "retry",
        "replayed": outcome.replayed,
        "stage": outcome.entry.stage,
        "state": outcome.entry.retry.state,
    }))
}

/// `agent.start` handler.
///
/// Args: `{ "name": "claude", "agent_type": "claude-code", "model": "sonnet"?, "model_present": true? }`
/// or `{ "name": "claude", "entry": AgentEntry }`.
/// Behaviour:
///   1. Validate `name` (non-empty) and resolve the requested
///      runtime type from top-level `agent_type` or `entry.agent_type`.
///   2. Load the registry. This daemon handler is the canonical
///      writer for `agents.json`: the CLI sends primitive fields,
///      while direct ability callers may supply a full `entry` v2
///      row for advanced import/migration flows.
///   3. If no `entry` is supplied, insert a minimal
///      `AgentEntry::new(agent_type, model)` when missing and leave
///      an existing row untouched. This preserves the programmatic
///      `agent.start` contract for direct ability callers.
///   4. Register `<name>.{chat,discover,invoke}` into `LocalRuntime`
///      via the `HotAgentRegistrar` so subsequent Axon-routed
///      dispatch lands ledger rows (Phase 5c).
///   5. Return the Agent URA plus the runtime registration counters.
fn start_agent_handler(
    args: Value,
    hot_registrar: &SharedHotRegistrarCell,
) -> anyhow::Result<Value> {
    let response = {
        let _mutation_guard = AgentLifecycleMutationGuard::acquire().map_err(|error| {
            anyhow::anyhow!("agent.start: acquire lifecycle transaction: {error:#}")
        })?;
        recover_pending_purge_local_locked(hot_registrar)?;
        start_agent_locked(args, hot_registrar)?
    };
    schedule_scheduled_purge_publications(hot_registrar)?;
    Ok(response)
}

fn start_agent_locked(
    args: Value,
    hot_registrar: &SharedHotRegistrarCell,
) -> anyhow::Result<Value> {
    let requested_name = args
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("agent.start: `name` (non-empty string) required"))?
        .to_string();
    // DEC-F048: hosted user agent ≠ device-sponsored System Agent.
    if crate::daemon::axon_bridge::hot_agent_registrar::name_claims_reserved_device_owner(
        &requested_name,
    ) {
        anyhow::bail!(
            "agent.start: `device.` is the reserved owner token for \
             device-sponsored System Agents (RFC-005 §3.1.2, DEC-F048); \
             hosted user agents cannot take a device-owned identity — \
             choose a name that is not `device` and does not begin with `device.`"
        );
    }
    let agent_id = AgentId::parse(&requested_name)
        .map_err(|error| anyhow::anyhow!("agent.start: invalid `name`: {error}"))?;
    let name = agent_id.name.clone();
    let registry_key = agent_id.to_string();
    if let Some(pending) = lifecycle_store::load_publication_outbox()?
        .entries
        .into_iter()
        .find(|entry| entry.name == name)
    {
        anyhow::bail!(
            "agent.start: `{name}` still has durable purge publication transaction `{}` pending; identity reuse is fenced until the tombstone/revoke outbox drains",
            pending.transaction_id
        );
    }
    let model = args
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string);
    let label = args
        .get("label")
        .and_then(Value::as_str)
        .map(str::to_string);
    let custom_command = args
        .get("command")
        .and_then(Value::as_str)
        .map(str::to_string);
    let custom_args: Vec<String> = args
        .get("command_args")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let materialize_directory = args
        .get("materialize_directory")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let update_existing_spec = args
        .get("update_existing_spec")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let model_present = match args.get("model_present") {
        Some(value) => value
            .as_bool()
            .ok_or_else(|| anyhow::anyhow!("agent.start: `model_present` must be a boolean"))?,
        None if args.get("model").is_some() => anyhow::bail!(
            "agent.start: `model_present` is required when `model` is supplied; \
             declare whether this invocation mutates the agent spec model"
        ),
        None => false,
    };
    let project_workspace = args
        .get("project_workspace")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let requested_root = args
        .get("root_path")
        .and_then(Value::as_str)
        .map(std::path::PathBuf::from);
    let provided_entry: Option<AgentEntry> = args
        .get("entry")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|err| anyhow::anyhow!("agent.start: invalid `entry`: {err}"))?;
    let top_level_agent_type = args
        .get("agent_type")
        .and_then(Value::as_str)
        .map(AgentType::from_str)
        .transpose()?;
    let entry_agent_type = provided_entry.as_ref().map(|entry| entry.agent_type);
    let agent_type = match (top_level_agent_type, entry_agent_type) {
        (Some(top_level), Some(from_entry)) if top_level != from_entry => anyhow::bail!(
            "agent.start: top-level `agent_type` ({top_level}) does not match \
             `entry.agent_type` ({from_entry})"
        ),
        (Some(agent_type), _) => agent_type,
        (None, Some(agent_type)) => agent_type,
        (None, None) => anyhow::bail!(
            "agent.start: either top-level `agent_type` or `entry.agent_type` is required"
        ),
    };

    // Two real caller shapes reach this handler:
    //
    //   1. **CLI `easynet agent add`** - sends primitive fields plus
    //      materialization flags. The daemon creates/updates
    //      `agent.toml`, writes `agents.json`, and syncs runtime rows;
    //      the CLI no longer mutates those files directly.
    //   2. **Direct `agent.start` invocation** (rare:
    //      programmatic registration, integration tests) — may omit
    //      `entry`; in that case we keep an existing row or create a
    //      minimal one.
    let registrar = require_hot_registrar(hot_registrar, "agent.start")?;
    let original_registry = agents::load_agents()
        .map_err(|error| anyhow::anyhow!("agent.start: load durable agent registry: {error:#}"))?;
    let original_local_agents = local_agents::load_for_fresh_host_projection()
        .map_err(|error| anyhow::anyhow!("agent.start: load hosted-Agent identities: {error:#}"))?;
    let mut transaction = AgentLifecycleTransaction::for_start(
        "agent.start",
        original_registry.clone(),
        original_local_agents.clone(),
    );
    let mut registry = original_registry;
    let existing_entry = registry.agents.get(&registry_key).cloned();
    let replaced_prior = existing_entry.is_some();
    if agent_type == AgentType::External {
        let command = custom_command
            .as_deref()
            .or_else(|| provided_entry.as_ref().map(|entry| entry.command.as_str()))
            .or_else(|| existing_entry.as_ref().map(|entry| entry.command.as_str()))
            .unwrap_or_default();
        if command.is_empty() {
            anyhow::bail!(
                "agent.start: external agents require `command`; use \
                 `easynet agent add <name> --type external --command <program> [--arg ...]`"
            );
        }
    }

    let mut materialized_directory: Option<AgentDirectory> = None;
    let mut created_directory = false;
    let mut updated_spec = false;
    if materialize_directory {
        let root = requested_root
            .or_else(|| {
                existing_entry
                    .as_ref()
                    .and_then(|entry| entry.root_path.clone())
            })
            .unwrap_or_else(|| config::agents_root().join(&name));
        if root.join("agent.toml").exists() && !replaced_prior {
            anyhow::bail!(
                "agent root at {} already carries an 'agent.toml' but no registry row. \
                 Import it by hand (add a registry row pointing at this path) or remove \
                 the directory before running `agent add`.",
                root.display()
            );
        }

        let spec_path = root.join("agent.toml");
        let directory = if spec_path.exists() {
            let mut directory = AgentDirectory::open(&root)?;
            if update_existing_spec && model_present {
                let prior_bytes = std::fs::read(&spec_path).map_err(|error| {
                    anyhow::anyhow!(
                        "agent.start: snapshot {} before update: {error}",
                        spec_path.display()
                    )
                })?;
                directory.set_model(model.clone())?;
                transaction.record_materialization(MaterializationRollback::UpdatedSpec {
                    path: spec_path.clone(),
                    prior_bytes,
                })?;
                updated_spec = true;
            }
            directory
        } else {
            let mut spec = AgentSpec::new(&name, runtime_kind_from(agent_type));
            if model_present {
                spec.model = model.clone();
            }
            if let Some(label) = label.as_ref() {
                spec.description = Some(label.clone());
            }
            let root_preexisted = root.exists();
            let directory = AgentDirectory::create(&Location::Local { root: root.clone() }, spec)
                .map_err(|error| {
                    let rollback = MaterializationRollback::Created {
                        root: root.clone(),
                        root_preexisted,
                    };
                    match rollback.rollback() {
                        Ok(()) => anyhow::anyhow!(
                            "agent.start: materialize agent directory: {error:#}; rollback=completed"
                        ),
                        Err(rollback_error) => anyhow::anyhow!(
                            "agent.start: materialize agent directory: {error:#}; rollback=partial({rollback_error:#})"
                        ),
                    }
                })?;
            transaction.record_materialization(MaterializationRollback::Created {
                root,
                root_preexisted,
            })?;
            created_directory = true;
            directory
        };
        materialized_directory = Some(directory);
    }

    let mut entry = if let Some(entry) = provided_entry {
        entry
    } else if let Some(existing) = existing_entry.clone() {
        if materialize_directory {
            let mut updated = existing;
            updated.agent_type = agent_type;
            if model_present {
                updated.with_model(model.clone());
            }
            if label.is_some() {
                updated.with_label(label.clone());
            }
            updated
        } else {
            existing
        }
    } else {
        AgentEntry::new(agent_type, model.clone())
    };

    if let Some(directory) = materialized_directory.as_ref() {
        normalize_v2_entry(&mut entry);
        let canonical_root = match std::fs::canonicalize(directory.root()) {
            Ok(root) => root,
            Err(error) => {
                return Err(transaction
                    .failure_with_rollback(format!(
                        "canonicalize materialized agent root {}: {error}",
                        directory.root().display()
                    ))
                    .into());
            }
        };
        entry.root_path = Some(canonical_root);
    }
    if agent_type == AgentType::External {
        if let Some(command) = custom_command
            .as_ref()
            .or_else(|| existing_entry.as_ref().map(|entry| &entry.command))
            .filter(|command| !command.is_empty())
        {
            entry.command = command.clone();
        }
        if !custom_args.is_empty() {
            entry.args = custom_args.clone();
        } else if let Some(existing) = existing_entry.as_ref() {
            entry.args = existing.args.clone();
        }
        if entry.command.is_empty() {
            anyhow::bail!(
                "agent.start: external agents require `command`; use \
                 `easynet agent add <name> --type external --command <program> [--arg ...]`"
            );
        }
    }
    if label.is_some() {
        entry.with_label(label.clone());
    }
    registry.agents.insert(registry_key.clone(), entry.clone());
    let identities = match hosted_agents_for_registry(&registry, original_local_agents) {
        Ok(identities) => identities,
        Err(error) => {
            return Err(transaction
                .failure_with_rollback(format!("build hosted-Agent identity projection: {error:#}"))
                .into());
        }
    };
    let agent_ura = match hosted_agent_ura_from_file(&identities, &name) {
        Ok(agent_ura) => agent_ura,
        Err(error) => {
            return Err(transaction
                .failure_with_rollback(format!("resolve hosted-Agent identity: {error:#}"))
                .into());
        }
    };
    transaction.persist(&registry, &identities)?;

    let name_for_registrar = name.clone();
    let entry_for_registrar = entry.clone();
    let previous_for_registrar = existing_entry.clone();
    let registrar_for_sync = Arc::clone(&registrar);
    let runtime_sync_outcome = block_on_hot_registrar(async move {
        registrar_for_sync
            .register_agent_replacing(
                &name_for_registrar,
                &entry_for_registrar,
                previous_for_registrar.as_ref(),
            )
            .await
    });
    let runtime_sync_outcome = match runtime_sync_outcome {
        Ok(outcome) => outcome,
        Err(error) => {
            return Err(transaction
                .failure_with_rollback(format!("synchronize authority/catalog/runtime: {error}"))
                .into());
        }
    };
    transaction.mark_runtime_synchronized()?;
    transaction.commit()?;

    let mut workspace_projected = false;
    let mut workspace_projection_error: Option<String> = None;
    if project_workspace {
        if let Some(directory) = materialized_directory.as_ref() {
            match crate::daemon::execution::mission::workspace::ensure_from_directory(directory) {
                Ok(_) => workspace_projected = true,
                Err(err) => workspace_projection_error = Some(format!("{err:#}")),
            }
        }
    }

    let runtime_registered = runtime_sync_outcome.registered;
    let runtime_replaced = runtime_sync_outcome.replaced;
    let runtime_failed = runtime_sync_outcome.failed;
    let runtime_removed = runtime_sync_outcome.removed;
    let runtime_not_ready = false;
    let runtime_catalog_not_ready = false;

    // ISS-002 closed loop: persist this hot-added agent's owner
    // projection into the cursor file NOW. Previously agent.start only
    // advertised the agent identity (advertise_hosted_agent) and never
    // built the ability projection, so the owner_ura never landed in
    // `owner-projections.json` — it was therefore absent from
    // `heartbeat_refresh_owner_uras`, its 60s lease was never renewed,
    // and the hub sweeper dropped the chat ability after expiry. With
    // lease cancelled (lease=0) projections no longer expire, and
    // persisting the cursor here makes the owner event-driven instead of
    // boot-only. Best-effort: a cursor write failure degrades to
    // "advertise still attempts" + an op_event, never blocks agent.start.
    let owner_projection_snapshot = registrar.publication_snapshot();
    let owner_projection_descriptors = owner_projection_snapshot
        .as_ref()
        .map(|snapshot| snapshot.owner_descriptors(&agent_ura))
        .unwrap_or_default();
    let mut abilities_payload: Option<Vec<u8>> = None;
    let mut owner_generation: Option<u64> = None;
    let mut owner_projection_state = if owner_projection_snapshot.is_ok() {
        "not_configured"
    } else {
        "failed"
    };
    let mut owner_projection_error = owner_projection_snapshot
        .as_ref()
        .err()
        .map(|error| format!("capture committed ability publication: {error}"));
    if owner_projection_snapshot.is_ok() {
        if let Some(host_device_ura) = config::load_credentials()
            .ok()
            .map(|creds| crate::core::ura::device_ura(creds.realm.trim(), creds.node_id.trim()))
            .filter(|ura| !ura.is_empty())
        {
            match crate::daemon::federation::read_model::owner_projection::prepare_and_persist(
                &agent_ura,
                &host_device_ura,
                &owner_projection_descriptors,
            ) {
                Ok(publication) => {
                    owner_projection_state = "persisted";
                    owner_generation = Some(publication.generation);
                    // Persisted to the cursor → owner now appears in
                    // `heartbeat_refresh_owner_uras`. Also build the wire
                    // payload so the advertiser pushes it to the hub NOW
                    // (event-driven), not at the next heartbeat. ISS-002.
                    match crate::daemon::federation::advertise::advertise_abilities_payload(
                        &agent_ura,
                        &publication,
                    )
                    .and_then(|payload| {
                        serde_json::to_vec(&payload)
                            .map_err(|e| format!("encode advertise_abilities payload: {e}"))
                    }) {
                        Ok(bytes) => abilities_payload = Some(bytes),
                        Err(err) => {
                            owner_projection_error = Some(err.clone());
                            crate::op_event!(
                                component = agent_lifecycle,
                                kind = hot_agent_abilities_payload_build_failed,
                                agent_name = name.as_str(),
                                agent_ura = agent_ura.as_str(),
                                error = err.as_str(),
                                message = "owner projection persisted but advertise payload \
                                       build failed; hub learns abilities on next \
                                       heartbeat refresh instead",
                            );
                        }
                    }
                }
                Err(err) => {
                    owner_projection_state = "failed";
                    owner_projection_error = Some(err.clone());
                    crate::op_event!(
                        component = agent_lifecycle,
                        kind = hot_agent_owner_projection_persist_failed,
                        agent_name = name.as_str(),
                        agent_ura = agent_ura.as_str(),
                        error = err.as_str(),
                        message = "agent registered but owner projection cursor was not \
                                   persisted; abilities resolvable locally but may lag in \
                                   the hub directory until next boot republish",
                    );
                }
            }
        }
    }

    let hub_advertise_scheduled = match (registrar.hot_agent_advertiser(), owner_generation) {
        (Some(advertiser), Some(generation)) => {
            let request = HotAgentAdvertiseRequest {
                agent_ura: agent_ura.clone(),
                generation,
                abilities_payload: abilities_payload.clone(),
            };
            schedule_hot_agent_advertise_retry(
                advertiser,
                request,
                name.clone(),
                agent_ura.clone(),
            );
            true
        }
        _ => false,
    };
    if hub_advertise_scheduled {
        crate::op_event!(
            component = agent_lifecycle,
            kind = hot_agent_hub_advertise_scheduled,
            agent_name = name.as_str(),
            agent_ura = agent_ura.as_str(),
            message = "agent registered locally; hub advertise will run through the session publication worker",
        );
    }

    Ok(json!({
        "agent_ura": agent_ura,
        "replaced_prior": replaced_prior,
        "runtime_registered": runtime_registered,
        "runtime_replaced": runtime_replaced,
        "runtime_failed": runtime_failed,
        "runtime_removed": runtime_removed,
        "runtime_not_ready": runtime_not_ready,
        "runtime_catalog_not_ready": runtime_catalog_not_ready,
        "hub_advertised": false,
        "hub_advertise_state": if hub_advertise_scheduled { "scheduled" } else { "not_configured" },
        "hub_advertise_error": Value::Null,
        "owner_projection_state": owner_projection_state,
        "owner_projection_error": owner_projection_error,
        "created_directory": created_directory,
        "updated_spec": updated_spec,
        "workspace_projected": workspace_projected,
        "workspace_projection_error": workspace_projection_error,
        "root_path": entry.root_path.as_ref().map(|path| path.to_string_lossy().to_string()),
        "agent_type": entry.agent_type.to_string(),
        "model": entry.model.clone(),
        "entry": entry.clone(),
    }))
}

fn runtime_kind_from(t: AgentType) -> RuntimeKind {
    t.runtime_kind()
}

#[cfg(not(test))]
const HOT_AGENT_ADVERTISE_RETRY_DELAYS_MS: &[u64] = &[500, 1_000, 2_000, 5_000, 10_000];

#[cfg(test)]
const HOT_AGENT_ADVERTISE_RETRY_DELAYS_MS: &[u64] = &[0, 0, 0];

fn schedule_hot_agent_advertise_retry(
    advertiser: Arc<dyn HotAgentAdvertiser>,
    request: HotAgentAdvertiseRequest,
    agent_name: String,
    agent_ura: String,
) {
    let retry_agent_name = agent_name.clone();
    let retry_agent_ura = agent_ura.clone();
    let spawn_result = std::thread::Builder::new()
        .name(format!("easynet-hot-agent-advertise-{agent_name}"))
        .spawn(move || {
            let mut last_error = String::new();
            for (index, delay_ms) in HOT_AGENT_ADVERTISE_RETRY_DELAYS_MS.iter().enumerate() {
                if *delay_ms > 0 {
                    std::thread::sleep(Duration::from_millis(*delay_ms));
                }
                let attempt = index + 1;
                let outcome = advertiser.advertise_hosted_agent(request.clone());
                if outcome.advertised() {
                    crate::op_event!(
                        component = agent_lifecycle,
                        kind = hot_agent_hub_advertise_retry_succeeded,
                        agent_name = retry_agent_name.as_str(),
                        agent_ura = retry_agent_ura.as_str(),
                        attempt = attempt,
                    );
                    return;
                }
                last_error = outcome.error().unwrap_or("unknown advertise failure").to_string();
                crate::op_event!(
                    component = agent_lifecycle,
                    kind = hot_agent_hub_advertise_retry_failed,
                    level = "warn",
                    agent_name = retry_agent_name.as_str(),
                    agent_ura = retry_agent_ura.as_str(),
                    attempt = attempt,
                    error = last_error.as_str(),
                );
            }
            crate::op_event!(
                component = agent_lifecycle,
                kind = hot_agent_hub_advertise_retry_exhausted,
                level = "warn",
                agent_name = retry_agent_name.as_str(),
                agent_ura = retry_agent_ura.as_str(),
                attempts = HOT_AGENT_ADVERTISE_RETRY_DELAYS_MS.len(),
                error = last_error.as_str(),
                message = "agent remains locally registered; hub discovery will recover on the next successful explicit advertise",
            );
        });

    if let Err(error) = spawn_result {
        let error = error.to_string();
        crate::op_event!(
            component = agent_lifecycle,
            kind = hot_agent_hub_advertise_retry_spawn_failed,
            level = "warn",
            agent_name = agent_name.as_str(),
            agent_ura = agent_ura.as_str(),
            error = error.as_str(),
        );
    }
}

fn schedule_hot_agent_projection_publication(
    advertiser: Arc<dyn HotAgentAdvertiser>,
    request: HotAgentProjectionRequest,
    agent_name: String,
    agent_ura: String,
) {
    let worker_agent_name = agent_name.clone();
    let worker_agent_ura = agent_ura.clone();
    let spawn_result = std::thread::Builder::new()
        .name(format!("easynet-hot-agent-project-{agent_name}"))
        .spawn(move || {
            let mut last_error = String::new();
            for (index, delay_ms) in HOT_AGENT_ADVERTISE_RETRY_DELAYS_MS.iter().enumerate() {
                if *delay_ms > 0 {
                    std::thread::sleep(Duration::from_millis(*delay_ms));
                }
                let attempt = index + 1;
                let outcome = advertiser.publish_owner_projection(request.clone());
                if outcome.advertised() {
                    crate::op_event!(
                        component = agent_lifecycle,
                        kind = hot_agent_projection_publication_succeeded,
                        agent_name = worker_agent_name.as_str(),
                        agent_ura = worker_agent_ura.as_str(),
                        attempt = attempt,
                    );
                    return;
                }
                last_error = outcome
                    .error()
                    .unwrap_or("unknown projection publication failure")
                    .to_string();
                crate::op_event!(
                    component = agent_lifecycle,
                    kind = hot_agent_projection_publication_failed,
                    level = "warn",
                    agent_name = worker_agent_name.as_str(),
                    agent_ura = worker_agent_ura.as_str(),
                    attempt = attempt,
                    error = last_error.as_str(),
                );
            }
            crate::op_event!(
                component = agent_lifecycle,
                kind = hot_agent_projection_publication_exhausted,
                level = "warn",
                agent_name = worker_agent_name.as_str(),
                agent_ura = worker_agent_ura.as_str(),
                attempts = HOT_AGENT_ADVERTISE_RETRY_DELAYS_MS.len(),
                error = last_error.as_str(),
                message = "agent lifecycle completed locally; hub projection publication remains pending reconciliation",
            );
        });

    if let Err(error) = spawn_result {
        let error = error.to_string();
        crate::op_event!(
            component = agent_lifecycle,
            kind = hot_agent_projection_publication_spawn_failed,
            level = "warn",
            agent_name = agent_name.as_str(),
            agent_ura = agent_ura.as_str(),
            error = error.as_str(),
        );
    }
}

fn schedule_hot_agent_revoke_publication(
    advertiser: Arc<dyn HotAgentAdvertiser>,
    request: HotAgentRevokeRequest,
    agent_name: String,
    agent_ura: String,
) {
    let worker_agent_name = agent_name.clone();
    let worker_agent_ura = agent_ura.clone();
    let spawn_result = std::thread::Builder::new()
        .name(format!("easynet-hot-agent-revoke-{agent_name}"))
        .spawn(move || {
            let mut last_error = String::new();
            for (index, delay_ms) in HOT_AGENT_ADVERTISE_RETRY_DELAYS_MS.iter().enumerate() {
                if *delay_ms > 0 {
                    std::thread::sleep(Duration::from_millis(*delay_ms));
                }
                let attempt = index + 1;
                let outcome = advertiser.revoke_hosted_agent(request.clone());
                if outcome.advertised() {
                    crate::op_event!(
                        component = agent_lifecycle,
                        kind = hot_agent_revoke_publication_succeeded,
                        agent_name = worker_agent_name.as_str(),
                        agent_ura = worker_agent_ura.as_str(),
                        attempt = attempt,
                    );
                    return;
                }
                last_error = outcome
                    .error()
                    .unwrap_or("unknown revoke publication failure")
                    .to_string();
                crate::op_event!(
                    component = agent_lifecycle,
                    kind = hot_agent_revoke_publication_failed,
                    level = "warn",
                    agent_name = worker_agent_name.as_str(),
                    agent_ura = worker_agent_ura.as_str(),
                    attempt = attempt,
                    error = last_error.as_str(),
                );
            }
            crate::op_event!(
                component = agent_lifecycle,
                kind = hot_agent_revoke_publication_exhausted,
                level = "warn",
                agent_name = worker_agent_name.as_str(),
                agent_ura = worker_agent_ura.as_str(),
                attempts = HOT_AGENT_ADVERTISE_RETRY_DELAYS_MS.len(),
                error = last_error.as_str(),
                message = "agent lifecycle completed locally; hub identity revoke remains pending reconciliation",
            );
        });

    if let Err(error) = spawn_result {
        let error = error.to_string();
        crate::op_event!(
            component = agent_lifecycle,
            kind = hot_agent_revoke_publication_spawn_failed,
            level = "warn",
            agent_name = agent_name.as_str(),
            agent_ura = agent_ura.as_str(),
            error = error.as_str(),
        );
    }
}

fn normalize_v2_entry(entry: &mut AgentEntry) {
    entry.command.clear();
    entry.args.clear();
    entry.env.clear();
    entry.timeout_secs = agents::default_timeout_for_new_rows();
    entry.max_output_bytes = agents::default_max_output_for_new_rows();
    entry.schema_version = CURRENT_REGISTRY_SCHEMA;
}

#[derive(Debug)]
struct PurgeFinalizeOutcome {
    state: &'static str,
    root: std::path::PathBuf,
}

fn advance_purge_journal(
    journal: &mut AgentPurgeJournal,
    stage: AgentPurgeStage,
) -> anyhow::Result<()> {
    journal.advance(stage)?;
    maybe_inject_purge_crash(stage)
}

#[cfg(not(test))]
fn maybe_inject_purge_crash(_stage: AgentPurgeStage) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
thread_local! {
    static PURGE_CRASH_STAGE: std::cell::Cell<Option<AgentPurgeStage>> = const { std::cell::Cell::new(None) };
    static PURGE_PRE_RENAME_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce(&std::path::Path)>>> =
        std::cell::RefCell::new(None);
    static PURGE_PRE_FINALIZE_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce(&std::path::Path)>>> =
        std::cell::RefCell::new(None);
    static PURGE_CHILD_ENTRY_HOOK: std::cell::RefCell<Option<Box<dyn FnMut(&std::ffi::OsStr) -> bool>>> =
        std::cell::RefCell::new(None);
    static PURGE_AFTER_TOMBSTONE_PUBLISH_CRASH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static PURGE_AFTER_REVOKE_PUBLISH_CRASH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn maybe_inject_purge_crash(stage: AgentPurgeStage) -> anyhow::Result<()> {
    if PURGE_CRASH_STAGE.with(|slot| slot.get() == Some(stage)) {
        anyhow::bail!("injected Agent purge crash after {stage:?}");
    }
    Ok(())
}

fn is_injected_purge_crash(error: &anyhow::Error) -> bool {
    error.to_string().contains("injected Agent purge crash")
}

#[derive(Clone, Copy)]
enum QuarantineValidation {
    FullIdentity,
    CommittedFinalize,
}

#[cfg(not(unix))]
#[derive(Debug, thiserror::Error)]
#[error(
    "agent.purge is unsupported on target `{target}`: identity-bound recursive deletion is unavailable"
)]
struct PurgePlatformUnsupported {
    target: &'static str,
}

struct PlatformTreeDeletion;

impl PlatformTreeDeletion {
    #[cfg(unix)]
    fn require_supported() -> anyhow::Result<()> {
        Ok(())
    }

    #[cfg(not(unix))]
    fn require_supported() -> anyhow::Result<()> {
        Err(PurgePlatformUnsupported {
            target: std::env::consts::OS,
        }
        .into())
    }
}

#[cfg(test)]
fn run_purge_pre_rename_hook(root: &std::path::Path) {
    if let Some(hook) = PURGE_PRE_RENAME_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook(root);
    }
}

#[cfg(not(test))]
fn run_purge_pre_rename_hook(_root: &std::path::Path) {}

#[cfg(test)]
fn run_purge_pre_finalize_hook(quarantine: &std::path::Path) {
    if let Some(hook) = PURGE_PRE_FINALIZE_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook(quarantine);
    }
}

#[cfg(not(test))]
fn run_purge_pre_finalize_hook(_quarantine: &std::path::Path) {}

#[cfg(test)]
fn run_purge_child_entry_hook(name: &std::ffi::OsStr) {
    PURGE_CHILD_ENTRY_HOOK.with(|slot| {
        let mut hook = slot.borrow_mut();
        let consumed = hook.as_mut().is_some_and(|hook| hook(name));
        if consumed {
            hook.take();
        }
    });
}

#[cfg(not(test))]
fn run_purge_child_entry_hook(_name: &std::ffi::OsStr) {}

#[cfg(test)]
fn maybe_inject_after_tombstone_publication() -> anyhow::Result<()> {
    if PURGE_AFTER_TOMBSTONE_PUBLISH_CRASH.with(|slot| slot.get()) {
        anyhow::bail!("injected Agent purge crash after Hub tombstone publication");
    }
    Ok(())
}

#[cfg(not(test))]
fn maybe_inject_after_tombstone_publication() -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
fn maybe_inject_after_revoke_publication() -> anyhow::Result<()> {
    if PURGE_AFTER_REVOKE_PUBLISH_CRASH.with(|slot| slot.get()) {
        anyhow::bail!("injected Agent purge crash after Hub revoke publication");
    }
    Ok(())
}

#[cfg(not(test))]
fn maybe_inject_after_revoke_publication() -> anyhow::Result<()> {
    Ok(())
}

fn validate_quarantined_agent_root(
    name: &str,
    root: &std::path::Path,
    quarantine: &std::path::Path,
    expected_identity: &AgentRootIdentity,
    open_root: Option<&std::fs::File>,
    validation: QuarantineValidation,
) -> anyhow::Result<()> {
    let expected_parent = root.parent().ok_or_else(|| {
        anyhow::anyhow!(
            "agent.purge: registered root {} has no parent",
            root.display()
        )
    })?;
    let quarantine_parent = quarantine.parent().ok_or_else(|| {
        anyhow::anyhow!(
            "agent.purge: quarantine {} has no parent",
            quarantine.display()
        )
    })?;
    let canonical_parent = std::fs::canonicalize(quarantine_parent).map_err(|error| {
        anyhow::anyhow!(
            "agent.purge: canonicalize quarantine parent {}: {error}",
            quarantine_parent.display()
        )
    })?;
    if quarantine_parent != expected_parent {
        anyhow::bail!(
            "agent.purge: quarantine parent mismatch: registered={}, quarantine={}",
            expected_parent.display(),
            quarantine_parent.display()
        );
    }
    #[cfg(unix)]
    if canonical_parent != expected_parent {
        anyhow::bail!(
            "agent.purge: quarantine parent is not canonical: expected={}, canonical={}",
            expected_parent.display(),
            canonical_parent.display()
        );
    }
    let metadata = std::fs::symlink_metadata(quarantine).map_err(|error| {
        anyhow::anyhow!(
            "agent.purge: inspect quarantine {}: {error}",
            quarantine.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        anyhow::bail!(
            "agent.purge: quarantine {} is not a real directory",
            quarantine.display()
        );
    }
    let canonical_quarantine = std::fs::canonicalize(quarantine)?;
    #[cfg(unix)]
    if canonical_quarantine != quarantine {
        anyhow::bail!(
            "agent.purge: quarantine {} is not canonical (canonical {})",
            quarantine.display(),
            canonical_quarantine.display()
        );
    }
    if !expected_identity.matches_path(quarantine)? {
        anyhow::bail!(
            "agent.purge: quarantine {} metadata identity differs from the registered root",
            quarantine.display()
        );
    }
    if let Some(handle) = open_root {
        let handle_metadata = handle.metadata()?;
        if !expected_identity.matches_metadata(&handle_metadata) {
            anyhow::bail!(
                "agent.purge: open root handle identity changed before quarantine validation"
            );
        }
    }
    if matches!(validation, QuarantineValidation::FullIdentity) {
        let directory = AgentDirectory::open(quarantine).map_err(|error| {
            anyhow::anyhow!(
                "agent.purge: quarantined root {} is not an agent directory: {error:#}",
                quarantine.display()
            )
        })?;
        if directory.spec().name != name {
            anyhow::bail!(
                "agent.purge: quarantined root {} belongs to `{}`, not `{name}`",
                quarantine.display(),
                directory.spec().name
            );
        }
    }
    Ok(())
}

fn restore_quarantine_atomically(
    root: &std::path::Path,
    quarantine: &std::path::Path,
) -> anyhow::Result<()> {
    if std::fs::symlink_metadata(root).is_ok() {
        anyhow::bail!(
            "agent.purge: cannot restore quarantine because registered root {} already exists",
            root.display()
        );
    }
    std::fs::rename(quarantine, root).map_err(|error| {
        anyhow::anyhow!(
            "agent.purge: restore quarantine {} to {}: {error}",
            quarantine.display(),
            root.display()
        )
    })?;
    config::sync_parent_dir(root)
        .map_err(|error| anyhow::anyhow!("sync restored purge root parent: {error:#}"))
}

fn quarantine_registered_root(journal: &mut AgentPurgeJournal) -> anyhow::Result<()> {
    match validate_registered_agent_root(&journal.name, &journal.root_path)? {
        RegisteredAgentRoot::Absent(root) => {
            journal.root_path = root;
            journal.root_identity = None;
        }
        RegisteredAgentRoot::Present(open_root) => {
            journal.root_path = open_root.root.clone();
            journal.root_identity = Some(open_root.identity.clone());
            lifecycle_store::save_purge_journal(journal)?;
            maybe_inject_purge_crash(AgentPurgeStage::Prepared)?;
            run_purge_pre_rename_hook(&journal.root_path);
            std::fs::rename(&journal.root_path, &journal.quarantine_path).map_err(|error| {
                anyhow::anyhow!(
                    "agent.purge: quarantine {} as {}: {error}",
                    journal.root_path.display(),
                    journal.quarantine_path.display()
                )
            })?;
            config::sync_parent_dir(&journal.quarantine_path)?;
            if let Err(error) = validate_quarantined_agent_root(
                &journal.name,
                &journal.root_path,
                &journal.quarantine_path,
                &open_root.identity,
                open_root.handle.as_ref(),
                QuarantineValidation::FullIdentity,
            ) {
                let restore =
                    restore_quarantine_atomically(&journal.root_path, &journal.quarantine_path);
                return match restore {
                    Ok(()) => Err(error.context("post-rename quarantine validation failed; root restored")),
                    Err(restore_error) => Err(anyhow::anyhow!(
                        "post-rename quarantine validation failed: {error:#}; restore failed: {restore_error:#}; residual_path={}",
                        journal.quarantine_path.display()
                    )),
                };
            }
        }
    }
    advance_purge_journal(journal, AgentPurgeStage::Quarantined)
}

#[derive(Debug)]
enum PurgeRecoveryStatus {
    Complete,
    PublicationPending(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PurgePublicationRetryTrigger {
    Scheduled,
    ConnectivityReady,
}

/// Recover local lifecycle state before boot reads transport credentials or
/// replays hosted agents. Committed transactions finish deletion and durable
/// outbox handoff here; external publication is owned by later transport boot.
pub(crate) fn recover_pending_purge_before_agent_replay(
    hot_registrar: &SharedHotRegistrarCell,
) -> anyhow::Result<bool> {
    let _mutation_guard = AgentLifecycleMutationGuard::acquire().map_err(|error| {
        anyhow::anyhow!("agent.purge boot recovery: acquire lifecycle transaction: {error:#}")
    })?;
    let Some(mut journal) = lifecycle_store::load_purge_journal()? else {
        return Ok(false);
    };
    if journal.stage.is_committed() {
        roll_forward_committed_purge(&mut journal)?;
        lifecycle_store::clear_purge_journal()?;
        return Ok(true);
    }
    rollback_uncommitted_purge(&journal, hot_registrar)?;
    Ok(true)
}

/// Refresh the daemon-local hosted identity projection from the Agent
/// bootstrap plan during startup.
///
/// Startup needs this projection before the daemon replays hosted agents, but
/// the write still belongs to the Agent lifecycle aggregate. Keeping the
/// mutation here means `cli start` can prepare the plan without becoming a
/// second `local-agents.json` writer.
pub(crate) fn bootstrap_local_agent_projection(
    plan: &BootstrapPlan,
) -> anyhow::Result<Vec<bootstrap::BootstrapOutcome>> {
    let _mutation_guard = AgentLifecycleMutationGuard::acquire().map_err(|error| {
        anyhow::anyhow!("agent.bootstrap: acquire lifecycle transaction: {error:#}")
    })?;
    let mut identities = local_agents::load_for_fresh_host_projection()
        .map_err(|error| anyhow::anyhow!("agent.bootstrap: load hosted identities: {error:#}"))?;
    let outcomes = bootstrap::bootstrap_local_agents(plan, &mut identities, &UuidMinter);
    AgentLifecycleProjectionStore::default()
        .persist_identities(&identities)
        .map_err(|error| {
            anyhow::anyhow!("agent.bootstrap: persist hosted identities: {error:#}")
        })?;
    Ok(outcomes)
}

/// Resume committed publication/finalization during boot. Hub transport may
/// not yet have a live session; a failed publication remains discoverable in
/// the journal and boot continues so the session can reconnect.
pub(crate) fn recover_pending_purge_on_boot(
    hot_registrar: &SharedHotRegistrarCell,
) -> anyhow::Result<bool> {
    {
        let _mutation_guard = AgentLifecycleMutationGuard::acquire().map_err(|error| {
            anyhow::anyhow!("agent.purge boot recovery: acquire lifecycle transaction: {error:#}")
        })?;
        recover_pending_purge_local_locked(hot_registrar)?;
    }
    match publication_recovery_status(
        hot_registrar,
        PurgePublicationRetryTrigger::ConnectivityReady,
    )? {
        PurgeRecoveryStatus::Complete => Ok(true),
        PurgeRecoveryStatus::PublicationPending(reason) => {
            crate::op_event!(
                component = agent_lifecycle,
                kind = purge_roll_forward_pending,
                level = "warn",
                error = reason.as_str(),
                journal = lifecycle_store::journal_path()
                    .display()
                    .to_string()
                    .as_str(),
                message = "Agent purge is locally complete; durable publication remains queued",
            );
            Ok(false)
        }
    }
}

fn recover_pending_purge_local_locked(
    hot_registrar: &SharedHotRegistrarCell,
) -> anyhow::Result<bool> {
    let Some(mut journal) = lifecycle_store::load_purge_journal()? else {
        return Ok(false);
    };
    if !journal.stage.is_committed() {
        rollback_uncommitted_purge(&journal, hot_registrar)?;
        return Ok(true);
    }
    roll_forward_committed_purge(&mut journal)?;
    lifecycle_store::clear_purge_journal()?;
    Ok(true)
}

fn publication_recovery_status(
    hot_registrar: &SharedHotRegistrarCell,
    trigger: PurgePublicationRetryTrigger,
) -> anyhow::Result<PurgeRecoveryStatus> {
    match drain_purge_publication_outbox(hot_registrar, trigger)? {
        None => Ok(PurgeRecoveryStatus::Complete),
        Some(reason) => Ok(PurgeRecoveryStatus::PublicationPending(reason)),
    }
}

#[cfg(not(test))]
fn schedule_scheduled_purge_publications(
    hot_registrar: &SharedHotRegistrarCell,
) -> anyhow::Result<()> {
    let pending = lifecycle_store::load_publication_outbox()?;
    if pending.entries.is_empty() {
        return Ok(());
    }
    let registrar = match require_hot_registrar(hot_registrar, "agent.purge.publish.schedule") {
        Ok(registrar) => registrar,
        Err(error) => {
            let error = error.to_string();
            crate::op_event!(
                component = agent_lifecycle,
                kind = purge_publication_schedule_deferred,
                level = "warn",
                error = error.as_str(),
                outbox = lifecycle_store::publication_outbox_path()
                    .display()
                    .to_string()
                    .as_str(),
                message = "local lifecycle mutation completed; purge publication remains queued until a publisher is available",
            );
            return Ok(());
        }
    };
    let spawn_result = std::thread::Builder::new()
        .name("easynet-agent-purge-publication-drain".to_string())
        .spawn(move || {
            let worker_cell = SharedHotRegistrarCell::new();
            if worker_cell.set(registrar).is_err() {
                crate::op_event!(
                    component = agent_lifecycle,
                    kind = purge_publication_schedule_failed,
                    level = "warn",
                    message = "failed to bind scheduled purge publication worker registrar",
                );
                return;
            }
            match publication_recovery_status(&worker_cell, PurgePublicationRetryTrigger::Scheduled)
            {
                Ok(PurgeRecoveryStatus::Complete) => crate::op_event!(
                    component = agent_lifecycle,
                    kind = purge_publication_scheduled_drain_completed,
                ),
                Ok(PurgeRecoveryStatus::PublicationPending(reason)) => crate::op_event!(
                    component = agent_lifecycle,
                    kind = purge_publication_scheduled_drain_pending,
                    level = "warn",
                    error = reason.as_str(),
                    outbox = lifecycle_store::publication_outbox_path()
                        .display()
                        .to_string()
                        .as_str(),
                    message = "local lifecycle mutation completed; purge publication remains queued for the next recovery trigger",
                ),
                Err(error) => {
                    let error = error.to_string();
                    crate::op_event!(
                        component = agent_lifecycle,
                        kind = purge_publication_scheduled_drain_failed,
                        level = "warn",
                        error = error.as_str(),
                        outbox = lifecycle_store::publication_outbox_path()
                            .display()
                            .to_string()
                            .as_str(),
                        message = "local lifecycle mutation completed; purge publication remains queued for the next recovery trigger",
                    );
                }
            }
        });
    if let Err(error) = spawn_result {
        let error = error.to_string();
        crate::op_event!(
            component = agent_lifecycle,
            kind = purge_publication_schedule_spawn_failed,
            level = "warn",
            error = error.as_str(),
            outbox = lifecycle_store::publication_outbox_path()
                .display()
                .to_string()
                .as_str(),
            message = "local lifecycle mutation completed; purge publication remains queued for the next recovery trigger",
        );
    }
    Ok(())
}

#[cfg(test)]
fn schedule_scheduled_purge_publications(
    _hot_registrar: &SharedHotRegistrarCell,
) -> anyhow::Result<()> {
    Ok(())
}

fn rollback_uncommitted_purge(
    journal: &AgentPurgeJournal,
    hot_registrar: &SharedHotRegistrarCell,
) -> anyhow::Result<()> {
    let registrar = require_hot_registrar(hot_registrar, "agent.purge.recover")?;
    restore_uncommitted_purge_root(journal)?;
    AgentLifecycleProjectionStore::default().restore_uncommitted_purge_snapshots(journal)?;
    let registrar_for_restore = Arc::clone(&registrar);
    let name = journal.name.clone();
    let entry = journal.removed_entry.clone();
    block_on_hot_registrar(
        async move { registrar_for_restore.register_agent(&name, &entry).await },
    )
    .map_err(|error| anyhow::anyhow!("recover Agent purge runtime/authority: {error}"))?;
    lifecycle_store::clear_purge_journal()
}

fn restore_uncommitted_purge_root(journal: &AgentPurgeJournal) -> anyhow::Result<()> {
    let root_exists = std::fs::symlink_metadata(&journal.root_path).is_ok();
    let quarantine_exists = std::fs::symlink_metadata(&journal.quarantine_path).is_ok();
    match (&journal.root_identity, root_exists, quarantine_exists) {
        (None, false, false) => Ok(()),
        (Some(identity), false, true) => {
            validate_quarantined_agent_root(
                &journal.name,
                &journal.root_path,
                &journal.quarantine_path,
                identity,
                None,
                QuarantineValidation::FullIdentity,
            )?;
            restore_quarantine_atomically(&journal.root_path, &journal.quarantine_path)
        }
        (Some(identity), true, false) => {
            if !identity.matches_path(&journal.root_path)? {
                anyhow::bail!(
                    "agent.purge recovery: registered root metadata changed at {}",
                    journal.root_path.display()
                );
            }
            Ok(())
        }
        _ => anyhow::bail!(
            "agent.purge recovery is ambiguous: root_exists={root_exists}, quarantine_exists={quarantine_exists}, journal={}",
            lifecycle_store::journal_path().display()
        ),
    }
}

#[cfg(unix)]
impl PlatformTreeDeletion {
    fn remove_quarantined_directory_identity_bound(
        quarantine: &std::path::Path,
        expected_identity: &AgentRootIdentity,
    ) -> anyhow::Result<()> {
        use rustix::fs::{openat, unlinkat, AtFlags, Mode, OFlags, CWD};

        let parent = quarantine.parent().ok_or_else(|| {
            anyhow::anyhow!(
                "identity-bound purge path {} has no parent",
                quarantine.display()
            )
        })?;
        let name = quarantine.file_name().ok_or_else(|| {
            anyhow::anyhow!(
                "identity-bound purge path {} has no basename",
                quarantine.display()
            )
        })?;
        let parent_fd = openat(
            CWD,
            parent,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| anyhow::anyhow!("open purge parent {}: {error}", parent.display()))?;
        let claimed_fd = openat(
            &parent_fd,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| anyhow::anyhow!("open claimed purge directory: {error}"))?;
        let claimed = std::fs::File::from(claimed_fd);
        let claimed_metadata = claimed.metadata()?;
        if !expected_identity.matches_metadata(&claimed_metadata) {
            anyhow::bail!(
                "claimed purge directory identity changed before descriptor-bound deletion"
            );
        }

        run_purge_pre_finalize_hook(quarantine);
        remove_open_directory_contents(&claimed)?;

        // Re-open the name immediately before unlinkat. If an attacker moved the
        // claimed inode while its descriptor was being drained, the replacement
        // path is preserved and recovery reports the residual instead of deleting
        // a different tree.
        let current_fd = openat(
            &parent_fd,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| anyhow::anyhow!("re-open claimed purge directory: {error}"))?;
        let current = std::fs::File::from(current_fd);
        if !expected_identity.matches_metadata(&current.metadata()?) {
            anyhow::bail!("claimed purge directory identity changed before unlinkat");
        }
        unlinkat(&parent_fd, name, AtFlags::REMOVEDIR)
            .map_err(|error| anyhow::anyhow!("unlinkat claimed purge directory: {error}"))
    }
}

#[cfg(unix)]
fn remove_open_directory_contents(directory: &std::fs::File) -> anyhow::Result<()> {
    use rustix::fs::{openat, statat, unlinkat, AtFlags, Dir, FileType, Mode, OFlags};
    use std::os::unix::ffi::OsStrExt as _;

    let entries = Dir::read_from(directory)
        .map_err(|error| anyhow::anyhow!("read claimed purge directory: {error}"))?;
    for entry in entries {
        let entry = entry.map_err(|error| anyhow::anyhow!("read purge entry: {error}"))?;
        let name = entry.file_name();
        if matches!(name.to_bytes(), b"." | b"..") {
            continue;
        }
        let stat = statat(directory, name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|error| anyhow::anyhow!("inspect purge entry {:?}: {error}", name))?;
        let initial_identity = UnixDirectoryEntryIdentity::from_stat(&stat);
        run_purge_child_entry_hook(std::ffi::OsStr::from_bytes(name.to_bytes()));
        if FileType::from_raw_mode(stat.st_mode) == FileType::Directory {
            let child_fd = openat(
                directory,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| anyhow::anyhow!("open purge child {:?}: {error}", name))?;
            let child = std::fs::File::from(child_fd);
            if !initial_identity.matches_metadata(&child.metadata()?) {
                anyhow::bail!(
                    "purge child {:?} changed identity between statat and openat",
                    name
                );
            }
            remove_open_directory_contents(&child)?;
            let current_fd = openat(
                directory,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| anyhow::anyhow!("re-open purge child {:?}: {error}", name))?;
            let current = std::fs::File::from(current_fd);
            if !initial_identity.matches_metadata(&current.metadata()?) {
                anyhow::bail!("purge child {:?} changed identity before unlinkat", name);
            }
            unlinkat(directory, name, AtFlags::REMOVEDIR)
                .map_err(|error| anyhow::anyhow!("unlinkat purge child {:?}: {error}", name))?;
        } else {
            let current = statat(directory, name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|error| anyhow::anyhow!("re-inspect purge entry {:?}: {error}", name))?;
            if !initial_identity.matches_stat(&current) {
                anyhow::bail!("purge entry {:?} changed identity before unlinkat", name);
            }
            unlinkat(directory, name, AtFlags::empty())
                .map_err(|error| anyhow::anyhow!("unlinkat purge entry {:?}: {error}", name))?;
        }
    }
    Ok(())
}

#[cfg(unix)]
#[derive(Clone, Copy)]
struct UnixDirectoryEntryIdentity {
    device: u64,
    inode: u64,
    file_type: u32,
}

#[cfg(unix)]
impl UnixDirectoryEntryIdentity {
    fn from_stat(stat: &rustix::fs::Stat) -> Self {
        Self {
            device: stat.st_dev as u64,
            inode: stat.st_ino as u64,
            file_type: stat.st_mode as u32 & libc::S_IFMT as u32,
        }
    }

    fn matches_stat(self, stat: &rustix::fs::Stat) -> bool {
        self.device == stat.st_dev as u64
            && self.inode == stat.st_ino as u64
            && self.file_type == (stat.st_mode as u32 & libc::S_IFMT as u32)
    }

    fn matches_metadata(self, metadata: &std::fs::Metadata) -> bool {
        use std::os::unix::fs::MetadataExt as _;
        self.device == metadata.dev()
            && self.inode == metadata.ino()
            && self.file_type == (metadata.mode() & libc::S_IFMT as u32)
    }
}

#[cfg(not(unix))]
impl PlatformTreeDeletion {
    fn remove_quarantined_directory_identity_bound(
        _quarantine: &std::path::Path,
        _expected_identity: &AgentRootIdentity,
    ) -> anyhow::Result<()> {
        Self::require_supported()
    }
}

fn finalize_committed_purge(journal: &AgentPurgeJournal) -> anyhow::Result<PurgeFinalizeOutcome> {
    let registry = agents::load_agents()?;
    let registry_key = canonical_agent_registry_key(&journal.name, "agent.purge.finalize")?;
    if registry.agents.contains_key(&registry_key) {
        anyhow::bail!(
            "agent.purge committed journal conflicts with agents.json row `{}`",
            registry_key
        );
    }
    let identities = local_agents::load_for_fresh_host_projection()?;
    if local_agents::lookup_hosted_ura(&identities, "llm", &journal.name).is_some() {
        anyhow::bail!(
            "agent.purge committed journal conflicts with local-agents.json entry `{}`",
            journal.name
        );
    }
    let root_exists = std::fs::symlink_metadata(&journal.root_path).is_ok();
    let quarantine_exists = std::fs::symlink_metadata(&journal.quarantine_path).is_ok();
    if root_exists {
        anyhow::bail!(
            "agent.purge committed journal cannot finalize because root reappeared at {}",
            journal.root_path.display()
        );
    }
    if quarantine_exists {
        let identity = journal.root_identity.as_ref().ok_or_else(|| {
            anyhow::anyhow!("agent.purge quarantine exists without a journaled root identity")
        })?;
        validate_quarantined_agent_root(
            &journal.name,
            &journal.root_path,
            &journal.quarantine_path,
            identity,
            None,
            QuarantineValidation::CommittedFinalize,
        )?;
        PlatformTreeDeletion::remove_quarantined_directory_identity_bound(
            &journal.quarantine_path,
            identity,
        )
        .map_err(|error| {
            anyhow::anyhow!(
                "agent.purge: delete committed quarantine {}: {error}; residual_path={}",
                journal.quarantine_path.display(),
                journal.quarantine_path.display()
            )
        })?;
        config::sync_parent_dir(&journal.quarantine_path)?;
    }
    Ok(PurgeFinalizeOutcome {
        state: if journal.root_identity.is_some() {
            "purged"
        } else {
            "already_absent"
        },
        root: journal.root_path.clone(),
    })
}

fn roll_forward_committed_purge(
    journal: &mut AgentPurgeJournal,
) -> anyhow::Result<PurgeFinalizeOutcome> {
    if !journal.stage.is_committed() {
        anyhow::bail!(
            "agent.purge: roll-forward requires committed journal, got {:?}",
            journal.stage
        );
    }

    if journal.stage == AgentPurgeStage::Committed {
        let outcome = finalize_committed_purge(journal)?;
        advance_purge_journal(journal, AgentPurgeStage::Finalized)?;
        maybe_inject_purge_crash(AgentPurgeStage::Finalized)?;
        if outcome.root != journal.root_path {
            anyhow::bail!("agent.purge: finalized root diverged from journaled root");
        }
    }

    if journal.stage == AgentPurgeStage::Finalized {
        journal.publication_plan = prepare_purge_tombstone_publication(journal)?;
        advance_purge_journal(journal, AgentPurgeStage::TombstonePrepared)?;
        maybe_inject_purge_crash(AgentPurgeStage::TombstonePrepared)?;
    }

    if journal.stage == AgentPurgeStage::TombstonePrepared {
        match &journal.publication_plan {
            AgentPurgePublicationPlan::Required { publication } => {
                enqueue_purge_publication(journal, publication)?;
                crate::daemon::federation::read_model::owner_projection::retire_removal_cursor(
                    &journal.agent_ura,
                    publication.projection_revision,
                    &publication.projection_digest,
                )
                .map_err(|error| anyhow::anyhow!("retire committed purge cursor: {error}"))?;
            }
            AgentPurgePublicationPlan::NotRequired { .. } => {}
            AgentPurgePublicationPlan::Undetermined => {
                anyhow::bail!(
                    "agent.purge: TombstonePrepared journal has undetermined publication plan"
                );
            }
        }
        advance_purge_journal(journal, AgentPurgeStage::OutboxEnqueued)?;
        maybe_inject_purge_crash(AgentPurgeStage::OutboxEnqueued)?;
    }

    if journal.stage == AgentPurgeStage::OutboxEnqueued {
        return Ok(PurgeFinalizeOutcome {
            state: if journal.root_identity.is_some() {
                "purged"
            } else {
                "already_absent"
            },
            root: journal.root_path.clone(),
        });
    }

    anyhow::bail!(
        "agent.purge: unsupported committed roll-forward stage {:?}",
        journal.stage
    )
}

fn enqueue_purge_publication(
    journal: &AgentPurgeJournal,
    publication: &AgentPurgePublication,
) -> anyhow::Result<()> {
    lifecycle_store::update_publication_outbox(|outbox| {
        if let Some(existing) = outbox
            .entries
            .iter()
            .find(|entry| entry.transaction_id == journal.transaction_id)
        {
            if existing.name != journal.name
                || existing.agent_ura != journal.agent_ura
                || existing.publication != *publication
            {
                anyhow::bail!(
                    "Agent purge publication transaction `{}` changed during replay",
                    journal.transaction_id
                );
            }
            return Ok(());
        }
        if let Some(existing) = outbox
            .entries
            .iter()
            .find(|entry| entry.agent_ura == journal.agent_ura)
        {
            anyhow::bail!(
                "Agent purge publication for `{}` is already pending as transaction `{}`",
                journal.agent_ura,
                existing.transaction_id
            );
        }
        outbox.entries.push(AgentPurgePublicationEntry {
            transaction_id: journal.transaction_id.clone(),
            name: journal.name.clone(),
            agent_ura: journal.agent_ura.clone(),
            publication: publication.clone(),
            stage: AgentPurgePublicationStage::TombstonePending,
            retry: AgentPurgePublicationRetry::default(),
            next_delivery_fence: 1,
        });
        outbox
            .entries
            .sort_by(|left, right| left.transaction_id.cmp(&right.transaction_id));
        Ok(())
    })
}

fn drain_purge_publication_outbox(
    hot_registrar: &SharedHotRegistrarCell,
    trigger: PurgePublicationRetryTrigger,
) -> anyhow::Result<Option<String>> {
    let Some(_drain_guard) = lifecycle_store::AgentPurgePublicationDrainGuard::try_acquire()?
    else {
        return Ok(Some(
            "purge publication drain is already active in this state directory".to_string(),
        ));
    };
    let drain_epoch =
        lifecycle_store::update_publication_outbox(|outbox| outbox.begin_drain_epoch())?;
    let advertiser = require_hot_registrar(hot_registrar, "agent.purge.publish")
        .map_err(|error| error.to_string())
        .and_then(|registrar| {
            registrar.hot_agent_advertiser().ok_or_else(|| {
                "Hub publisher is unavailable for durable purge publication".to_string()
            })
        });

    let mut attempted_transactions = std::collections::BTreeSet::new();
    for _ in 0..lifecycle_store::PUBLICATION_DRAIN_BATCH_SIZE {
        let now_unix_ms = purge_publication_now_unix_ms()?;
        let Some(mut entry) =
            claim_next_purge_publication(trigger, drain_epoch, &attempted_transactions)?
        else {
            break;
        };
        attempted_transactions.insert(entry.transaction_id.clone());
        let claim_id = entry
            .claim_id()
            .ok_or_else(|| anyhow::anyhow!("claimed purge publication has no claim ID"))?
            .to_string();
        let delivery_fence = entry
            .delivery_fence()
            .ok_or_else(|| anyhow::anyhow!("claimed purge publication has no delivery fence"))?;
        let advertiser = match &advertiser {
            Ok(advertiser) => advertiser,
            Err(error) => {
                record_purge_publication_failure(
                    &entry.transaction_id,
                    &claim_id,
                    now_unix_ms,
                    error.clone(),
                )?;
                continue;
            }
        };

        if entry.stage == AgentPurgePublicationStage::TombstonePending {
            let mut tombstone: crate::daemon::federation::read_model::owner_projection::OwnerProjectionPublication =
                serde_json::from_slice(&entry.publication.tombstone_payload).map_err(|error| {
                    anyhow::anyhow!(
                        "decode journaled purge tombstone `{}`: {error}",
                        entry.transaction_id
                    )
                })?;
            tombstone.purge_delivery = Some(
                crate::daemon::federation::read_model::owner_projection::PurgeProjectionDelivery {
                    protocol_version:
                        crate::daemon::persistence::federation_revoke::REVOKE_PROTOCOL_VERSION,
                    transaction_id: entry.transaction_id.clone(),
                    generation: entry.publication.generation,
                    authority_ura: entry.publication.host_device_ura.clone(),
                    delivery_fence,
                },
            );
            let tombstone_payload = serde_json::to_vec(&tombstone).map_err(|error| {
                anyhow::anyhow!(
                    "encode fenced purge tombstone `{}`: {error}",
                    entry.transaction_id
                )
            })?;
            let outcome = advertiser.publish_owner_projection(
                crate::daemon::axon_bridge::hot_agent_registrar::HotAgentProjectionRequest {
                    agent_ura: entry.agent_ura.clone(),
                    generation: entry.publication.generation,
                    transaction_id: entry.transaction_id.clone(),
                    delivery_fence,
                    abilities_payload: tombstone_payload,
                },
            );
            if outcome.state() != HotAgentAdvertiseState::Succeeded {
                record_purge_publication_failure(
                    &entry.transaction_id,
                    &claim_id,
                    now_unix_ms,
                    format!(
                        "Hub ability tombstone failed: {}",
                        outcome.error().unwrap_or("unknown publication failure")
                    ),
                )?;
                continue;
            }
            maybe_inject_after_tombstone_publication()?;
            entry = lifecycle_store::update_publication_outbox(|outbox| {
                let current = outbox
                    .entries
                    .iter_mut()
                    .find(|current| current.transaction_id == entry.transaction_id)
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "purge publication `{}` disappeared after tombstone",
                            entry.transaction_id
                        )
                    })?;
                current.advance_claim_to_revoke(&claim_id)?;
                Ok(current.clone())
            })?;
        }

        let outcome = advertiser.revoke_hosted_agent(
            crate::daemon::axon_bridge::hot_agent_registrar::HotAgentRevokeRequest {
                agent_ura: entry.agent_ura.clone(),
                generation: entry.publication.generation,
                reason: "agent.purge".to_string(),
                purge_transaction_id: Some(entry.transaction_id.clone()),
                authority_ura: entry.publication.host_device_ura.clone(),
                protocol_version:
                    crate::daemon::persistence::federation_revoke::REVOKE_PROTOCOL_VERSION,
                delivery_fence,
            },
        );
        if outcome.state() != HotAgentAdvertiseState::Succeeded {
            record_purge_publication_failure(
                &entry.transaction_id,
                &claim_id,
                now_unix_ms,
                format!(
                    "Hub Agent revoke failed: {}",
                    outcome.error().unwrap_or("unknown revoke failure")
                ),
            )?;
            continue;
        }
        maybe_inject_after_revoke_publication()?;
        lifecycle_store::update_publication_outbox(|outbox| {
            let index = outbox
                .entries
                .iter()
                .position(|current| current.transaction_id == entry.transaction_id)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "purge publication `{}` disappeared before completion",
                        entry.transaction_id
                    )
                })?;
            if outbox.entries[index].claim_id() != Some(claim_id.as_str()) {
                anyhow::bail!(
                    "purge publication `{}` completion lost its durable claim",
                    entry.transaction_id
                );
            }
            outbox.entries.remove(index);
            Ok(())
        })?;
    }

    let pending = lifecycle_store::load_publication_outbox()?;
    if pending.entries.is_empty() {
        return Ok(None);
    }
    let failed = pending
        .entries
        .iter()
        .filter_map(|entry| {
            entry.retry.last_failure.as_ref().map(|failure| {
                format!(
                    "{} transaction={} stage={:?}: {}",
                    entry.agent_ura, entry.transaction_id, failure.stage, failure.error
                )
            })
        })
        .collect::<Vec<_>>();
    let reconciliation_required = pending
        .entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.retry.state,
                lifecycle_store::AgentPurgePublicationRetryState::ReconciliationRequired { .. }
            )
        })
        .count();
    Ok(Some(if failed.is_empty() {
        format!(
            "{} purge publication transaction(s) remain pending; {} require reconciliation",
            pending.entries.len(),
            reconciliation_required
        )
    } else {
        failed.join("; ")
    }))
}

fn purge_publication_now_unix_ms() -> anyhow::Result<u64> {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| anyhow::anyhow!("system clock precedes Unix epoch: {error}"))?;
    u64::try_from(elapsed.as_millis())
        .map_err(|_| anyhow::anyhow!("system clock milliseconds exceed durable u64 range"))
}

fn claim_next_purge_publication(
    trigger: PurgePublicationRetryTrigger,
    drain_epoch: u64,
    excluded_transactions: &std::collections::BTreeSet<String>,
) -> anyhow::Result<Option<AgentPurgePublicationEntry>> {
    lifecycle_store::update_publication_outbox(|outbox| {
        for entry in &mut outbox.entries {
            if excluded_transactions.contains(&entry.transaction_id) {
                continue;
            }
            let claim_id = uuid::Uuid::new_v4().simple().to_string();
            if entry.claim(
                drain_epoch,
                trigger == PurgePublicationRetryTrigger::ConnectivityReady,
                claim_id,
            )? {
                return Ok(Some(entry.clone()));
            }
        }
        Ok(None)
    })
}

fn record_purge_publication_failure(
    transaction_id: &str,
    claim_id: &str,
    now_unix_ms: u64,
    error: String,
) -> anyhow::Result<()> {
    lifecycle_store::update_publication_outbox(|outbox| {
        let current = outbox
            .entries
            .iter_mut()
            .find(|current| current.transaction_id == transaction_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "purge publication `{}` disappeared while recording failure",
                    transaction_id
                )
            })?;
        current.record_claim_failure(claim_id, now_unix_ms, error)?;
        Ok(())
    })
}

fn prepare_purge_tombstone_publication(
    journal: &AgentPurgeJournal,
) -> anyhow::Result<AgentPurgePublicationPlan> {
    let publication_required =
        crate::daemon::federation::read_model::owner_projection::publication_required(
            &journal.agent_ura,
        )
        .map_err(anyhow::Error::msg)?;
    if !publication_required {
        return Ok(AgentPurgePublicationPlan::NotRequired {
            reason: AgentPurgeNoPublicationReason::NoActiveOwnerProjection,
        });
    }
    let Some(publication) =
        crate::daemon::federation::read_model::owner_projection::prepare_journaled_removal(
            &journal.agent_ura,
        )
        .map_err(|error| anyhow::anyhow!("prepare committed purge tombstone: {error}"))?
    else {
        return Ok(AgentPurgePublicationPlan::NotRequired {
            reason: AgentPurgeNoPublicationReason::NoActiveOwnerProjection,
        });
    };
    let payload = serde_json::to_vec(&publication)
        .map_err(|error| anyhow::anyhow!("encode committed purge tombstone: {error}"))?;
    Ok(AgentPurgePublicationPlan::Required {
        publication: AgentPurgePublication {
            host_device_ura: publication.host_device_ura,
            generation: publication.generation,
            projection_revision: publication.projection_revision,
            projection_digest: publication.projection_digest,
            tombstone_payload: payload,
        },
    })
}

fn compensate_nonjournaled_removal(
    transaction: &mut AgentLifecycleTransaction,
    registrar: &Arc<HotAgentRegistrar>,
    name: &str,
    entry: &AgentEntry,
    cause: anyhow::Error,
) -> anyhow::Error {
    let mut failures = transaction.rollback();
    let registrar_for_restore = Arc::clone(registrar);
    let name_for_restore = name.to_string();
    let entry_for_restore = entry.clone();
    if let Err(error) = block_on_hot_registrar(async move {
        registrar_for_restore
            .register_agent(&name_for_restore, &entry_for_restore)
            .await
    }) {
        failures.push(format!("restore runtime/catalog/authority: {error}"));
    }
    AgentLifecycleError::Mutation {
        operation: "agent.stop",
        state: if failures.is_empty() {
            AgentLifecycleState::RolledBack
        } else {
            AgentLifecycleState::PartialFailure
        },
        cause: cause.to_string(),
        rollback: if failures.is_empty() {
            "completed".to_string()
        } else {
            format!("partial({})", failures.join("; "))
        },
    }
    .into()
}

fn purge_agent_handler(
    args: Value,
    hot_registrar: &SharedHotRegistrarCell,
) -> anyhow::Result<Value> {
    PlatformTreeDeletion::require_supported()?;
    let mut committed = {
        let _mutation_guard = AgentLifecycleMutationGuard::acquire().map_err(|error| {
            anyhow::anyhow!("agent.purge: acquire lifecycle transaction: {error:#}")
        })?;
        recover_pending_purge_local_locked(hot_registrar)?;
        purge_agent_locked(args, hot_registrar)?
    };
    if committed.transaction_id.is_some() {
        decorate_purge_publication_response(&mut committed)?;
        schedule_scheduled_purge_publications(hot_registrar)?;
    }
    Ok(committed.response)
}

struct CommittedPurgeResponse {
    response: Value,
    transaction_id: Option<String>,
    publication_required: bool,
}

fn purge_agent_locked(
    args: Value,
    hot_registrar: &SharedHotRegistrarCell,
) -> anyhow::Result<CommittedPurgeResponse> {
    let name = agent_name_from_lifecycle_args(&args, "agent.purge")?;
    let registry_key = canonical_agent_registry_key(&name, "agent.purge")?;
    let original_registry = agents::load_agents()
        .map_err(|error| anyhow::anyhow!("agent.purge: load agents.json: {error:#}"))?;
    let Some(removed_entry) = original_registry.agents.get(&registry_key).cloned() else {
        return Ok(CommittedPurgeResponse {
            response: json!({
                "ack": false,
                "purge_state": "not_applicable",
                "purged_path": Value::Null,
                "runtime_removed": 0,
                "removed_entry": Value::Null,
            }),
            transaction_id: None,
            publication_required: false,
        });
    };
    let root_path = removed_entry.root_path.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "agent.purge: registered agent `{name}` has no `root_path`; refusing to infer a destructive path"
        )
    })?;
    let parent = root_path.parent().ok_or_else(|| {
        anyhow::anyhow!(
            "agent.purge: registered root {} has no parent",
            root_path.display()
        )
    })?;
    let original_local_agents = local_agents::load_for_fresh_host_projection()
        .map_err(|error| anyhow::anyhow!("agent.purge: load local-agents.json: {error:#}"))?;
    let agent_ura = hosted_agent_ura_from_file(&original_local_agents, &name)
        .map_err(|error| anyhow::anyhow!("agent.purge: {error:#}"))?;
    let transaction_id = uuid::Uuid::new_v4().simple().to_string();
    let quarantine_path = parent.join(format!(".{name}.easynet-purge-{transaction_id}"));
    match std::fs::symlink_metadata(&quarantine_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) => anyhow::bail!(
            "agent.purge: unique quarantine path already exists at {}; refusing overwrite",
            quarantine_path.display()
        ),
        Err(error) => anyhow::bail!(
            "agent.purge: inspect quarantine candidate {}: {error}",
            quarantine_path.display()
        ),
    }
    let mut journal = AgentPurgeJournal::new(
        transaction_id,
        name.clone(),
        agent_ura,
        root_path,
        quarantine_path,
        removed_entry,
        original_registry,
        original_local_agents,
    );

    if let Err(error) = quarantine_registered_root(&mut journal) {
        if is_injected_purge_crash(&error) {
            return Err(error);
        }
        let rollback = recover_pending_purge_local_locked(hot_registrar);
        return Err(match rollback {
            Ok(_) => anyhow::anyhow!(
                "agent.purge preparation failed: {error:#}; rollback=completed"
            ),
            Err(rollback_error) => anyhow::anyhow!(
                "agent.purge preparation failed: {error:#}; rollback failed: {rollback_error:#}; journal={}",
                lifecycle_store::journal_path().display()
            ),
        });
    }

    let removal = stop_agent_locked(
        json!({"name": name}),
        hot_registrar,
        "agent.purge",
        Some(&mut journal),
    );
    let mut response = match removal {
        Ok(response) => response,
        Err(error) if is_injected_purge_crash(&error) => return Err(error),
        Err(error) => {
            let rollback = recover_pending_purge_local_locked(hot_registrar);
            return Err(match rollback {
                Ok(_) => anyhow::anyhow!(
                    "agent.purge application transaction failed: {error:#}; rollback=completed"
                ),
                Err(rollback_error) => anyhow::anyhow!(
                    "agent.purge failed: {error:#}; recovery failed: {rollback_error:#}; journal={}",
                    lifecycle_store::journal_path().display()
                ),
            });
        }
    };

    let outcome = roll_forward_committed_purge(&mut journal)?;
    lifecycle_store::clear_purge_journal()?;
    let publication_required = matches!(
        &journal.publication_plan,
        AgentPurgePublicationPlan::Required { .. }
    );
    let object = response
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("agent.purge internal response is not an object"))?;
    object.insert("purge_state".to_string(), json!(outcome.state));
    object.insert(
        "purged_path".to_string(),
        json!(outcome.root.to_string_lossy().to_string()),
    );
    Ok(CommittedPurgeResponse {
        response,
        transaction_id: Some(journal.transaction_id),
        publication_required,
    })
}

fn decorate_purge_publication_response(
    committed: &mut CommittedPurgeResponse,
) -> anyhow::Result<()> {
    let transaction_id = committed
        .transaction_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("committed purge response has no transaction ID"))?;
    let pending_outbox = lifecycle_store::load_publication_outbox()?;
    let own_pending_publication = pending_outbox
        .entries
        .iter()
        .find(|entry| entry.transaction_id == transaction_id);
    let publication_pending = own_pending_publication.is_some();
    let reconciliation_required = own_pending_publication.is_some_and(|entry| {
        matches!(
            entry.retry.state,
            lifecycle_store::AgentPurgePublicationRetryState::ReconciliationRequired { .. }
        )
    });
    let own_publication_error = own_pending_publication.map(|entry| {
        entry
            .retry
            .last_failure
            .as_ref()
            .map(|failure| failure.error.clone())
            .unwrap_or_else(|| "publication is durably queued for retry".to_string())
    });
    let object = committed
        .response
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("agent.purge internal response is not an object"))?;
    object.insert(
        "publication_state".to_string(),
        json!(if !committed.publication_required {
            "not_applicable"
        } else if reconciliation_required {
            "reconciliation_required"
        } else if publication_pending {
            "pending"
        } else {
            "published"
        }),
    );
    object.insert(
        "publication_error".to_string(),
        json!(own_publication_error.clone()),
    );
    Ok(())
}

/// `agent.stop` handler.
///
/// Args: `{ "name": "claude" }` or
/// `{ "agent_ura": "easynet:///r/<realm>/agent/<user>.claude" }`.
/// Behaviour: remove the registry row while preserving its root directory.
/// Idempotent — `ack=false` if the row didn't exist.
fn stop_agent_handler(
    args: Value,
    hot_registrar: &SharedHotRegistrarCell,
) -> anyhow::Result<Value> {
    let response = {
        let _mutation_guard = AgentLifecycleMutationGuard::acquire().map_err(|error| {
            anyhow::anyhow!("agent.stop: acquire lifecycle transaction: {error:#}")
        })?;
        recover_pending_purge_local_locked(hot_registrar)?;
        stop_agent_locked(args, hot_registrar, "agent.stop", None)?
    };
    schedule_scheduled_purge_publications(hot_registrar)?;
    Ok(response)
}

fn stop_agent_locked(
    args: Value,
    hot_registrar: &SharedHotRegistrarCell,
    operation: &'static str,
    mut purge_journal: Option<&mut AgentPurgeJournal>,
) -> anyhow::Result<Value> {
    if args.get("purge").is_some() {
        anyhow::bail!("{operation}: `purge` is not accepted; invoke `agent.purge`");
    }
    let original_registry = agents::load_agents()
        .map_err(|error| anyhow::anyhow!("{operation}: load durable agent registry: {error:#}"))?;
    let name = agent_name_from_lifecycle_args(&args, operation)?;
    let registry_key = canonical_agent_registry_key(&name, operation)?;
    let Some(removed_entry) = original_registry.agents.get(&registry_key).cloned() else {
        return Ok(json!({
            "ack": false,
            "runtime_removed": 0,
            "removed_entry": Value::Null,
            "publication_state": "not_applicable",
            "publication_error": Value::Null,
        }));
    };
    let registrar = require_hot_registrar(hot_registrar, operation)?;
    let original_local_agents = local_agents::load_for_fresh_host_projection()
        .map_err(|error| anyhow::anyhow!("{operation}: load hosted-Agent identities: {error:#}"))?;
    let agent_ura = hosted_agent_ura_from_file(&original_local_agents, &name)
        .map_err(|error| anyhow::anyhow!("{operation}: {error:#}"))?;
    let mut transaction = AgentLifecycleTransaction::for_stop(
        operation,
        original_registry.clone(),
        original_local_agents.clone(),
    );

    let registrar_for_remove = Arc::clone(&registrar);
    let name_for_remove = name.clone();
    let entry_for_remove = removed_entry.clone();
    let removal = block_on_hot_registrar(async move {
        registrar_for_remove
            .unregister_agent(&name_for_remove, &entry_for_remove)
            .await
    });
    let removal = removal
        .map_err(|error| anyhow::anyhow!("synchronize authority/catalog/runtime removal: {error}"));
    let removal = match removal {
        Ok(removal) => removal,
        Err(error) if purge_journal.is_some() => return Err(error),
        Err(error) => {
            return Err(compensate_nonjournaled_removal(
                &mut transaction,
                &registrar,
                &name,
                &removed_entry,
                error,
            ));
        }
    };
    let runtime_removed = removal.outcome().removed;
    transaction.mark_runtime_synchronized()?;
    if let Some(journal) = purge_journal.as_deref_mut() {
        advance_purge_journal(journal, AgentPurgeStage::RuntimeSynchronized)?;
    }

    let mut registry = original_registry.clone();
    registry.agents.remove(&registry_key);
    let mut identities = original_local_agents.clone();
    identities
        .hosted_agents
        .retain(|entry| !(entry.profile == "llm" && entry.name == name));
    if let Err(error) = transaction.persist_registry_projection(&registry) {
        if purge_journal.is_some() {
            return Err(error);
        }
        return Err(compensate_nonjournaled_removal(
            &mut transaction,
            &registrar,
            &name,
            &removed_entry,
            error,
        ));
    }
    if let Some(journal) = purge_journal.as_deref_mut() {
        advance_purge_journal(journal, AgentPurgeStage::RegistryPersisted)?;
    }

    if let Err(error) = transaction.persist_identity_projection(&identities) {
        if purge_journal.is_some() {
            return Err(error);
        }
        return Err(compensate_nonjournaled_removal(
            &mut transaction,
            &registrar,
            &name,
            &removed_entry,
            error,
        ));
    }
    if let Some(journal) = purge_journal.as_deref_mut() {
        advance_purge_journal(journal, AgentPurgeStage::IdentityPersisted)?;
    }

    // Phase two proves both lifecycle rows are gone before revoking authority.
    if let Err(error) = registrar.commit_agent_removal(&removal) {
        let error = anyhow::anyhow!("revoke hosted-Agent authority: {error}");
        if purge_journal.is_some() {
            return Err(error);
        }
        return Err(compensate_nonjournaled_removal(
            &mut transaction,
            &registrar,
            &name,
            &removed_entry,
            error,
        ));
    }
    if let Some(journal) = purge_journal.as_deref_mut() {
        advance_purge_journal(journal, AgentPurgeStage::AuthorityCommitted)?;
    }
    transaction.mark_authority_synchronized()?;
    transaction.commit()?;
    if let Some(journal) = purge_journal.as_deref_mut() {
        advance_purge_journal(journal, AgentPurgeStage::Committed)?;
        return Ok(json!({
            "ack": true,
            "runtime_removed": runtime_removed,
            "removed_entry": removed_entry,
            "publication_state": "pending",
            "publication_error": Value::Null,
        }));
    }

    // ISS-002 closed loop (stop side, symmetric to start): tell the hub
    // the agent's abilities are gone NOW instead of waiting for the next
    // heartbeat. We advertise an empty complete-set so the hub's
    // complete-set REPLACE tombstones every prior projected ability
    // (removed = old − ∅), and we drop the local cursor so the owner
    // leaves the heartbeat refresh batch. Best-effort: failures degrade
    // to "reconciles on next boot/heartbeat" + an op_event.
    let mut publication_state = "not_applicable";
    let mut publication_error: Option<String> = None;
    let mut revoke_generation: Option<u64> = None;
    if let Some(host_device_ura) = config::load_credentials()
        .ok()
        .map(|creds| crate::core::ura::device_ura(creds.realm.trim(), creds.node_id.trim()))
        .filter(|ura| !ura.is_empty())
    {
        let advertiser = registrar.hot_agent_advertiser();

        // Step 1: tombstone the agent's abilities (empty complete-set
        // → hub removes all prior projected abilities) + drop the
        // local cursor so the owner leaves the heartbeat batch.
        match crate::daemon::federation::read_model::owner_projection::prepare_removal_and_persist(
            &agent_ura,
            &host_device_ura,
        ) {
            Ok(Some(publication)) => {
                revoke_generation = Some(publication.generation);
                let tombstone_payload =
                    crate::daemon::federation::advertise::advertise_abilities_payload(
                        &agent_ura,
                        &publication,
                    )
                    .and_then(|payload| {
                        serde_json::to_vec(&payload).map_err(|e| {
                            format!("encode advertise_abilities tombstone payload: {e}")
                        })
                    });
                match (tombstone_payload, advertiser.as_ref()) {
                    (Ok(payload), Some(advertiser)) => {
                        let transaction_id = purge_journal
                            .as_deref()
                            .map(|journal| journal.transaction_id.clone())
                            .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
                        schedule_hot_agent_projection_publication(
	                            Arc::clone(advertiser),
	                            crate::daemon::axon_bridge::hot_agent_registrar::HotAgentProjectionRequest {
	                                agent_ura: agent_ura.clone(),
	                                generation: publication.generation,
	                                transaction_id,
	                                delivery_fence: 1,
	                                abilities_payload: payload,
	                            },
	                            name.clone(),
	                            agent_ura.clone(),
	                        );
                        publication_state = "pending";
                    }
                    (Ok(_), None) => {
                        publication_state = "pending";
                    }
                    (Err(error), _) => {
                        publication_state = "reconciliation_required";
                        publication_error = Some(error);
                    }
                }
            }
            Ok(None) => publication_state = "not_applicable",
            Err(err) => {
                publication_state = "reconciliation_required";
                publication_error = Some(err.clone());
                crate::op_event!(
                    component = agent_lifecycle,
                    kind = hot_agent_stop_tombstone_build_failed,
                    agent_name = name.as_str(),
                    agent_ura = agent_ura.as_str(),
                    error = err.as_str(),
                    message = "agent stopped but owner projection tombstone could \
                                   not be built; hub reconciles on next heartbeat",
                );
            }
        }

        // Step 2: revoke the agent IDENTITY from the hub directory
        // (federation.revoke), symmetric to advertise_hosted_agent on
        // start. Without this the agent record lingers in the hub
        // catalogue after stop (with lease cancelled it would not age
        // out on its own). ISS-002.
        if let (Some(advertiser), Some(generation)) = (advertiser.as_ref(), revoke_generation) {
            schedule_hot_agent_revoke_publication(
                Arc::clone(advertiser),
                crate::daemon::axon_bridge::hot_agent_registrar::HotAgentRevokeRequest {
                    agent_ura: agent_ura.clone(),
                    generation,
                    reason: operation.to_string(),
                    purge_transaction_id: purge_journal
                        .as_deref()
                        .map(|journal| journal.transaction_id.clone()),
                    authority_ura: host_device_ura.clone(),
                    protocol_version:
                        crate::daemon::persistence::federation_revoke::REVOKE_PROTOCOL_VERSION,
                    delivery_fence: 1,
                },
                name.clone(),
                agent_ura.clone(),
            );
            if publication_state != "reconciliation_required" {
                publication_state = "pending";
            }
        }
    }

    Ok(json!({
        "ack": true,
        "runtime_removed": runtime_removed,
        "removed_entry": removed_entry,
        "publication_state": publication_state,
        "publication_error": publication_error,
    }))
}

/// `agent.refresh` handler.
///
/// Args: `{ "name": "claude"? }`.
/// Behaviour: re-register the selected agent, or every row in
/// `agents.json`, into the daemon-owned `LocalRuntime`. This is the
/// daemon-hosted replacement for the old CLI-side
/// runtime refresh sweep.
fn refresh_agents_handler(
    args: Value,
    hot_registrar: &SharedHotRegistrarCell,
) -> anyhow::Result<Value> {
    let response = {
        let _mutation_guard = AgentLifecycleMutationGuard::acquire().map_err(|error| {
            anyhow::anyhow!("agent.refresh: acquire lifecycle transaction: {error:#}")
        })?;
        recover_pending_purge_local_locked(hot_registrar)?;
        refresh_agents_locked(args, hot_registrar)?
    };
    schedule_scheduled_purge_publications(hot_registrar)?;
    Ok(response)
}

fn refresh_agents_locked(
    args: Value,
    hot_registrar: &SharedHotRegistrarCell,
) -> anyhow::Result<Value> {
    let requested_name = args
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let registry = agents::load_agents().map_err(|error| {
        anyhow::anyhow!("agent.refresh: load durable agent registry: {error:#}")
    })?;
    let rows: Vec<(String, AgentEntry)> = match requested_name.as_ref() {
        Some(name) => {
            let registry_key = canonical_agent_registry_key(name, "agent.refresh")?;
            let entry = registry.agents.get(&registry_key).cloned().ok_or_else(|| {
                anyhow::anyhow!("agent.refresh: agent {name:?} is not registered")
            })?;
            vec![(name.clone(), entry)]
        }
        None => registry
            .agents
            .iter()
            .map(|(key, entry)| {
                let agent_id = AgentId::parse(key).map_err(|error| {
                    anyhow::anyhow!("agent.refresh: invalid registry key {key:?}: {error}")
                })?;
                Ok((agent_id.name, entry.clone()))
            })
            .collect::<anyhow::Result<Vec<_>>>()?,
    };

    let registrar = require_hot_registrar(hot_registrar, "agent.refresh")?;
    let agent_results = block_on_hot_registrar(async move {
        let mut agent_results = Vec::with_capacity(rows.len());
        for (name, entry) in rows {
            let outcome = registrar.register_agent(&name, &entry).await?;
            agent_results.push(json!({
                "name": name,
                "runtime_registered": outcome.registered,
                "runtime_failed": outcome.failed,
                "runtime_removed": outcome.removed,
                "runtime_not_ready": false,
                "runtime_catalog_not_ready": false,
            }));
        }
        Ok::<_, HotAgentRegistrarError>(agent_results)
    })
    .map_err(|source| AgentLifecycleError::Registrar {
        operation: "agent.refresh",
        source: Box::new(source),
    })?;

    let runtime_registered = agent_results
        .iter()
        .filter_map(|row| row.get("runtime_registered").and_then(Value::as_u64))
        .sum::<u64>();
    let runtime_failed = agent_results
        .iter()
        .filter_map(|row| row.get("runtime_failed").and_then(Value::as_u64))
        .sum::<u64>();
    let runtime_removed = agent_results
        .iter()
        .filter_map(|row| row.get("runtime_removed").and_then(Value::as_u64))
        .sum::<u64>();
    Ok(json!({
        "ok": true,
        "runtime_not_ready": false,
        "runtime_catalog_not_ready": false,
        "agents_scanned": agent_results.len(),
        "runtime_registered": runtime_registered,
        "runtime_failed": runtime_failed,
        "runtime_removed": runtime_removed,
        "agents": agent_results,
    }))
}

fn hosted_agents_for_registry(
    registry: &AgentRegistry,
    mut file: local_agents::LocalAgentsFile,
) -> anyhow::Result<local_agents::LocalAgentsFile> {
    let plan = hosted_agent_bootstrap_plan(registry)?;
    bootstrap::bootstrap_local_agents(&plan, &mut file, &UuidMinter);
    Ok(file)
}

fn hosted_agent_ura_from_file(
    file: &local_agents::LocalAgentsFile,
    name: &str,
) -> anyhow::Result<String> {
    let mut matches = file
        .hosted_agents
        .iter()
        .filter(|entry| entry.profile == "llm" && entry.name == name);
    let entry = matches
        .next()
        .ok_or_else(|| anyhow::anyhow!("hosted Agent {name:?} has no llm identity row"))?;
    if matches.next().is_some() {
        anyhow::bail!("hosted Agent {name:?} has multiple llm identity rows");
    }
    let parsed = crate::core::ura::parse_ura(&entry.agent_ura).map_err(|error| {
        anyhow::anyhow!("invalid hosted Agent URA {:?}: {error}", entry.agent_ura)
    })?;
    let Some((_, agent_id)) = parsed.agent_ids() else {
        anyhow::bail!(
            "hosted Agent {name:?} identity {:?} is not a user-hosted Agent URA",
            entry.agent_ura
        );
    };
    if parsed.kind != crate::core::ura::URAKind::Agent || agent_id != name {
        anyhow::bail!(
            "hosted Agent {name:?} identity {:?} has a mismatched Agent id",
            entry.agent_ura
        );
    }
    Ok(entry.agent_ura.clone())
}

fn hosted_agent_bootstrap_plan(registry: &AgentRegistry) -> anyhow::Result<BootstrapPlan> {
    let credentials = config::load_credentials().map_err(|error| {
        anyhow::anyhow!(
            "agent.start requires joined credentials before deriving hosted-Agent identity: {error:#}"
        )
    })?;
    let realm = credentials.realm.trim().to_string();
    let node_id = credentials.node_id.trim().to_string();
    if realm.is_empty() || node_id.is_empty() {
        anyhow::bail!(
            "agent.start requires joined credentials with non-empty realm and Device node id"
        );
    }
    let user_id = credentials
        .user_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string();
    let host_device_ura = crate::core::ura::device_ura(&realm, &node_id);

    Ok(BootstrapPlan {
        realm,
        user_id,
        host_device_ura,
        consent: true,
        mcp: false,
        llm_sub_agents: bootstrap::llm_sub_agents_from_registry(registry)?,
    })
}

fn canonical_agent_registry_key(name: &str, operation: &'static str) -> anyhow::Result<String> {
    AgentId::parse(name)
        .map(|agent_id| agent_id.to_string())
        .map_err(|error| anyhow::anyhow!("{operation}: invalid agent name {name:?}: {error}"))
}

fn agent_name_from_lifecycle_args(args: &Value, operation: &'static str) -> anyhow::Result<String> {
    let name = args
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let agent_ura = args
        .get("agent_ura")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    match (name, agent_ura) {
        (Some(name), None) => Ok(name.to_string()),
        (None, Some(ura)) => agent_name_from_ura(ura, operation),
        (Some(name), Some(ura)) => {
            let from_ura = agent_name_from_ura(ura, operation)?;
            if from_ura != name {
                anyhow::bail!(
                    "{operation}: `name` ({name}) does not match `agent_ura` ({from_ura})"
                );
            }
            Ok(name.to_string())
        }
        (None, None) => {
            anyhow::bail!("{operation}: either `name` or `agent_ura` is required")
        }
    }
}

fn agent_name_from_ura(ura: &str, operation: &'static str) -> anyhow::Result<String> {
    let parsed = crate::core::ura::parse_ura(ura)
        .map_err(|err| anyhow::anyhow!("{operation}: invalid `agent_ura`: {err}"))?;
    if parsed.kind != crate::core::ura::URAKind::Agent {
        anyhow::bail!("{operation}: `agent_ura` must be an Agent URA");
    }
    // DEC-F048: device-sponsored System Agents are not hosted user
    // agents — they cannot be registered here (see the agent.start
    // gate), so a lifecycle reference to one is a category error,
    // not a missing-agent case.
    if parsed.device_agent_ids().is_some() {
        anyhow::bail!(
            "{operation}: {ura} is a device-sponsored System Agent \
             (RFC-005 §3.1.2, DEC-F048); System Agents are not \
             lifecycle-managed as hosted agents on this surface"
        );
    }
    let identities = crate::daemon::persistence::local_agents::load_for_fresh_host_projection()
        .map_err(|error| anyhow::anyhow!("{operation}: load hosted-Agent identities: {error:#}"))?;
    if let Some(entry) = identities
        .hosted_agents
        .into_iter()
        .find(|entry| entry.profile == "llm" && entry.agent_ura == ura)
    {
        return Ok(entry.name);
    }
    let Some((_, agent_id)) = parsed.agent_ids() else {
        anyhow::bail!("agent URA is missing agent_id");
    };
    Ok(agent_id.to_string())
}

// ── Discovery surfaces ────────────────────────────────────────

pub fn start_agent_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["name"],
        "dependentRequired": {
            "model": ["model_present"]
        },
        "anyOf": [
            { "required": ["agent_type"] },
            { "required": ["entry"] }
        ],
        "properties": {
            "name":       { "type": "string", "minLength": 1 },
            "agent_type": { "type": "string",
                            "enum": ["claude-code", "claude", "codex",
                                     "codex-app-server", "codex-appserver"] },
            "model":      { "type": ["string", "null"] },
            "model_present": { "type": "boolean" },
            "label":      { "type": "string" },
            "root_path":  { "type": "string" },
            "entry": agent_entry_input_schema(),
            "materialize_directory": { "type": "boolean" },
            "update_existing_spec": { "type": "boolean" },
            "project_workspace": { "type": "boolean" },
        },
        "additionalProperties": false,
    })
}

fn agent_entry_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["agent_type"],
        "properties": {
            "schema_version": { "type": "integer", "minimum": 0 },
            "root_path": { "type": ["string", "null"] },
            "agent_type": {
                "type": "string",
                "enum": ["claude-code", "claude", "codex", "codex-app-server", "codex-appserver"]
            },
            "command": { "type": "string" },
            "args": {
                "type": "array",
                "items": { "type": "string" }
            },
            "model": { "type": ["string", "null"] },
            "label": { "type": ["string", "null"] },
            "env": {
                "type": "object",
                "additionalProperties": { "type": "string" }
            },
            "timeout_secs": { "type": "integer", "minimum": 0 },
            "max_output_bytes": { "type": "integer", "minimum": 0 }
        },
        "additionalProperties": false
    })
}

pub fn start_agent_description() -> &'static str {
    "Register a new LLM sub-agent (claude/codex/…) in the device's \
     agent registry. This is the daemon-hosted surface behind \
     `easynet agent add`; callers may send primitive fields or a rich \
     entry object. Returns agent_ura and replaced_prior=true means the \
     call overwrote an existing row."
}

pub fn stop_agent_input_schema() -> Value {
    json!({
        "type": "object",
        "anyOf": [
            { "required": ["name"] },
            { "required": ["agent_ura"] }
        ],
        "properties": {
            "name": { "type": "string", "minLength": 1 },
            "agent_ura": { "type": "string", "minLength": 1 },
        },
        "additionalProperties": false,
    })
}

pub fn stop_agent_description() -> &'static str {
    "Remove an LLM sub-agent registry row by name or Agent URA. \
     Idempotent: ack=false when the row didn't exist. The registered root \
     directory is always preserved."
}

pub fn purge_agent_input_schema() -> Value {
    stop_agent_input_schema()
}

pub fn purge_agent_description() -> &'static str {
    "Destructively remove an LLM sub-agent and the exact canonical root_path \
     stored in its registry row. Requires Manage authority. The daemon commits \
     local state and identity-bound deletion before handing external Hub \
     publication to a durable retry outbox."
}

pub fn purge_reconcile_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["transaction_id", "command_id"],
        "properties": {
            "transaction_id": { "type": "string", "pattern": "^[0-9a-f]{32}$" },
            "command_id": { "type": "string", "pattern": "^[0-9a-f]{32}$" },
            "action": { "type": "string", "enum": ["retry"] }
        },
        "additionalProperties": false,
    })
}

pub fn purge_reconcile_description() -> &'static str {
    "Authorize and audit an idempotent retry of one dead-lettered Agent purge \
     publication transaction. The operation retains the identity fence and \
     terminal failure evidence."
}

pub fn refresh_agents_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": { "type": "string", "minLength": 1 },
        },
        "additionalProperties": false,
    })
}

pub fn refresh_agents_description() -> &'static str {
    "Re-register the LocalRuntime handlers for one or all LLM \
     sub-agents declared in agents.json. With `name` set, only that \
     agent's `<agent>.*` handlers and ability TOMLs are rebuilt; \
     with `name` omitted, every row in agents.json is rebuilt. This \
     is the daemon-hosted surface behind `easynet agent refresh` and \
     re-reads `<agent-root>/abilities/*.ability.toml` from disk."
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::ability::catalog::profiles::bootstrap::LlmSubAgent;

    const TEST_DEVICE_URA: &str = "easynet:///r/test/device/local";

    /// Test fixture: route `~/.easynet/` at a fresh tempdir for the
    /// duration of `f`. Uses the canonical `test_support::HomeGuard`
    /// — same fixture as registry::agents tests and the dispatch suite
    /// — which (a) acquires a process-global mutex so two HomeGuards
    /// never run concurrently and (b) sets `HOME` (the var
    /// `config::home_dir()` actually reads), not `EASYNET_HOME`.
    ///
    /// History: an earlier draft of this fixture set `EASYNET_HOME`
    /// (which `config::home_dir()` ignores) and only PID+nanos-keyed
    /// the tempdir, leaving the env-var racing under parallel tests.
    /// See `docs/rfc/AXON-RFC-001-flake-localization.md` (2026-04-27).
    fn with_isolated_home<F: FnOnce()>(f: F) {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        f();
    }

    fn ready_hot_registrar() -> SharedHotRegistrarCell {
        let advertiser: Arc<
            dyn crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiser,
        > = Arc::new(RecordingHotAdvertiser::default());
        ready_hot_registrar_fixture(Some(advertiser), "localhost").cell
    }

    fn seed_joined_credentials() {
        crate::daemon::persistence::config::save_credentials(
            &crate::daemon::persistence::config::Credentials {
                node_id: "dev-1".to_string(),
                credential_token: "token".to_string(),
                hub_endpoint: "axon://hub.test:50051".to_string(),
                realm: "localhost".to_string(),
                username: Some("dev".to_string()),
                user_id: Some("user-dev".to_string()),
                ..Default::default()
            },
        )
        .expect("seed joined credentials");
    }

    #[test]
    fn startup_bootstrap_projection_persists_through_lifecycle_owner() {
        with_isolated_home(|| {
            let plan = BootstrapPlan {
                realm: "local".to_string(),
                user_id: "user-dev".to_string(),
                host_device_ura: crate::core::ura::device_ura("local", "dev-1"),
                consent: false,
                mcp: false,
                llm_sub_agents: vec![LlmSubAgent {
                    name: "claude".to_string(),
                    agent_type_display: "claude-code".to_string(),
                    model: None,
                }],
            };

            let outcomes = bootstrap_local_agent_projection(&plan)
                .expect("startup bootstrap projection persists");
            let identities = local_agents::load().expect("load persisted hosted identities");

            assert_eq!(outcomes.len(), 1);
            assert_eq!(identities.host_device_agent_ura, plan.host_device_ura);
            assert_eq!(identities.hosted_agents.len(), 1);
            assert_eq!(identities.hosted_agents[0].profile, "llm");
            assert_eq!(identities.hosted_agents[0].name, "claude");
            assert_eq!(
                identities.hosted_agents[0].signing_authority,
                format!("hosted_by:{}", plan.host_device_ura)
            );
        });
    }

    #[derive(Default)]
    struct RecordingHotAdvertiser {
        requests: std::sync::Mutex<Vec<String>>,
        ability_payloads: std::sync::Mutex<Vec<Vec<u8>>>,
        revokes: std::sync::Mutex<Vec<String>>,
        revoke_transactions: std::sync::Mutex<Vec<Option<String>>>,
        hub_projection_fence: std::sync::Mutex<std::collections::BTreeMap<String, (u64, String)>>,
        idempotent_projection_replays: std::sync::atomic::AtomicUsize,
    }

    impl crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiser
        for RecordingHotAdvertiser
    {
        fn advertise_hosted_agent(
            &self,
            request: crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseRequest,
        ) -> crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome {
            self.requests.lock().unwrap().push(request.agent_ura);
            if let Some(payload) = request.abilities_payload {
                let value: serde_json::Value = match serde_json::from_slice(&payload) {
                    Ok(value) => value,
                    Err(error) => {
                        return crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome::failed(
                            format!("Hub rejected malformed owner projection: {error}"),
                        );
                    }
                };
                let owner = value["owner_ura"].as_str().unwrap_or_default().to_string();
                let revision = value["projection_revision"].as_u64().unwrap_or_default();
                let digest = value["projection_digest"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let mut fence = self.hub_projection_fence.lock().unwrap();
                if let Some((current_revision, current_digest)) = fence.get(&owner) {
                    if revision < *current_revision
                        || (revision == *current_revision && digest != *current_digest)
                    {
                        return crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome::failed(
                            format!(
                                "Hub revision fence rejected owner={owner} incoming=({revision}, {digest}) current=({current_revision}, {current_digest})"
                            ),
                        );
                    }
                    if revision == *current_revision {
                        self.idempotent_projection_replays
                            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                }
                fence.insert(owner, (revision, digest));
                drop(fence);
                self.ability_payloads.lock().unwrap().push(payload);
            }
            crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome::succeeded()
        }

        fn publish_owner_projection(
            &self,
            request: crate::daemon::axon_bridge::hot_agent_registrar::HotAgentProjectionRequest,
        ) -> crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome {
            let payload = request.abilities_payload;
            let value: serde_json::Value = match serde_json::from_slice(&payload) {
                Ok(value) => value,
                Err(error) => {
                    return crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome::failed(
                        format!("Hub rejected malformed owner projection: {error}"),
                    );
                }
            };
            let owner = value["owner_ura"].as_str().unwrap_or_default().to_string();
            let revision = value["projection_revision"].as_u64().unwrap_or_default();
            let digest = value["projection_digest"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let mut fence = self.hub_projection_fence.lock().unwrap();
            if let Some((current_revision, current_digest)) = fence.get(&owner) {
                if revision < *current_revision
                    || (revision == *current_revision && digest != *current_digest)
                {
                    return crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome::failed(
                        "Hub projection fence rejected tombstone",
                    );
                }
                if revision == *current_revision {
                    self.idempotent_projection_replays
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            }
            fence.insert(owner, (revision, digest));
            drop(fence);
            self.ability_payloads.lock().unwrap().push(payload);
            crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome::succeeded()
        }

        fn revoke_hosted_agent(
            &self,
            request: crate::daemon::axon_bridge::hot_agent_registrar::HotAgentRevokeRequest,
        ) -> crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome {
            self.revoke_transactions
                .lock()
                .unwrap()
                .push(request.purge_transaction_id.clone());
            self.revokes.lock().unwrap().push(request.agent_ura);
            crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome::succeeded()
        }
    }

    struct FailOnceHotAdvertiser {
        attempts: std::sync::atomic::AtomicUsize,
        succeeded: std::sync::Mutex<std::sync::mpsc::Sender<usize>>,
    }

    impl crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiser for FailOnceHotAdvertiser {
        fn advertise_hosted_agent(
            &self,
            _request: crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseRequest,
        ) -> crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome {
            let attempt = self
                .attempts
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 1;
            if attempt == 1 {
                return crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome::failed(
                    "injected first advertise timeout",
                );
            }
            self.succeeded.lock().unwrap().send(attempt).unwrap();
            crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome::succeeded()
        }

        fn publish_owner_projection(
            &self,
            _request: crate::daemon::axon_bridge::hot_agent_registrar::HotAgentProjectionRequest,
        ) -> crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome {
            crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome::succeeded()
        }

        fn revoke_hosted_agent(
            &self,
            _request: crate::daemon::axon_bridge::hot_agent_registrar::HotAgentRevokeRequest,
        ) -> crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome {
            crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome::succeeded()
        }
    }

    #[test]
    fn hot_agent_advertise_retry_replays_failed_publication() {
        let (tx, rx) = std::sync::mpsc::channel();
        let advertiser = Arc::new(FailOnceHotAdvertiser {
            attempts: std::sync::atomic::AtomicUsize::new(0),
            succeeded: std::sync::Mutex::new(tx),
        });
        let agent_ura = "easynet:///r/test/agent/alice.retry".to_string();

        schedule_hot_agent_advertise_retry(
            advertiser,
            crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseRequest {
                agent_ura: agent_ura.clone(),
                generation: 1,
                abilities_payload: Some(
                    br#"{"owner_ura":"easynet:///r/test/agent/alice.retry"}"#.to_vec(),
                ),
            },
            "retry".to_string(),
            agent_ura,
        );

        assert_eq!(
            rx.recv_timeout(Duration::from_secs(1))
                .expect("retry should reach success"),
            2
        );
    }

    #[derive(Default)]
    struct SelectiveFailureHotAdvertiser {
        recording: RecordingHotAdvertiser,
        tombstone_failures: std::sync::Mutex<std::collections::BTreeSet<String>>,
        revoke_failures: std::sync::Mutex<std::collections::BTreeSet<String>>,
    }

    struct BlockingHotAdvertiser {
        recording: RecordingHotAdvertiser,
        fail_tombstone: std::sync::atomic::AtomicBool,
        block_next_tombstone: std::sync::atomic::AtomicBool,
        started: std::sync::mpsc::Sender<()>,
        release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    }

    impl crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiser for BlockingHotAdvertiser {
        fn advertise_hosted_agent(
            &self,
            request: crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseRequest,
        ) -> crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome {
            if request.abilities_payload.is_some()
                && self
                    .fail_tombstone
                    .load(std::sync::atomic::Ordering::SeqCst)
            {
                return crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome::failed(
                    "hold publication in the outbox",
                );
            }
            if request.abilities_payload.is_some()
                && self
                    .block_next_tombstone
                    .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                self.started.send(()).unwrap();
                self.release
                    .lock()
                    .unwrap()
                    .recv()
                    .expect("test releases blocked publication");
            }
            self.recording.advertise_hosted_agent(request)
        }

        fn publish_owner_projection(
            &self,
            request: crate::daemon::axon_bridge::hot_agent_registrar::HotAgentProjectionRequest,
        ) -> crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome {
            if self
                .fail_tombstone
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                return crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome::failed(
                    "hold publication in the outbox",
                );
            }
            if self
                .block_next_tombstone
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                self.started.send(()).unwrap();
                self.release
                    .lock()
                    .unwrap()
                    .recv()
                    .expect("test releases blocked publication");
            }
            self.recording.publish_owner_projection(request)
        }

        fn revoke_hosted_agent(
            &self,
            request: crate::daemon::axon_bridge::hot_agent_registrar::HotAgentRevokeRequest,
        ) -> crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome {
            self.recording.revoke_hosted_agent(request)
        }
    }

    impl SelectiveFailureHotAdvertiser {
        fn fail_tombstone_for(&self, agent_ura: &str) {
            self.tombstone_failures
                .lock()
                .unwrap()
                .insert(agent_ura.to_string());
        }

        fn allow_tombstone_for(&self, agent_ura: &str) {
            self.tombstone_failures.lock().unwrap().remove(agent_ura);
        }

        fn fail_revoke_for(&self, agent_ura: &str) {
            self.revoke_failures
                .lock()
                .unwrap()
                .insert(agent_ura.to_string());
        }
    }

    impl crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiser
        for SelectiveFailureHotAdvertiser
    {
        fn advertise_hosted_agent(
            &self,
            request: crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseRequest,
        ) -> crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome {
            if self
                .tombstone_failures
                .lock()
                .unwrap()
                .contains(&request.agent_ura)
            {
                return crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome::failed(
                    format!("poisoned tombstone for {}", request.agent_ura),
                );
            }
            crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiser::advertise_hosted_agent(
                &self.recording,
                request,
            )
        }

        fn publish_owner_projection(
            &self,
            request: crate::daemon::axon_bridge::hot_agent_registrar::HotAgentProjectionRequest,
        ) -> crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome {
            if self
                .tombstone_failures
                .lock()
                .unwrap()
                .contains(&request.agent_ura)
            {
                return crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome::failed(
                    format!("poisoned tombstone for {}", request.agent_ura),
                );
            }
            self.recording.publish_owner_projection(request)
        }

        fn revoke_hosted_agent(
            &self,
            request: crate::daemon::axon_bridge::hot_agent_registrar::HotAgentRevokeRequest,
        ) -> crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome {
            if self
                .revoke_failures
                .lock()
                .unwrap()
                .contains(&request.agent_ura)
            {
                return crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome::failed(
                    format!("poisoned revoke for {}", request.agent_ura),
                );
            }
            crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiser::revoke_hosted_agent(
                &self.recording,
                request,
            )
        }
    }

    fn expire_publication_claim(transaction_id: &str) {
        lifecycle_store::update_publication_outbox(|outbox| {
            {
                let entry = outbox
                    .entries
                    .iter()
                    .find(|entry| entry.transaction_id == transaction_id)
                    .expect("publication entry exists while expiring test claim");
                assert!(
                    matches!(
                        entry.retry.state,
                        lifecycle_store::AgentPurgePublicationRetryState::Claimed { .. }
                    ),
                    "publication entry must hold a claim"
                );
            }
            outbox.begin_drain_epoch()?;
            Ok(())
        })
        .unwrap();
    }

    fn hot_registrar_with_advertiser(
        advertiser: Arc<RecordingHotAdvertiser>,
    ) -> SharedHotRegistrarCell {
        let advertiser: Arc<
            dyn crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiser,
        > = advertiser;
        ready_hot_registrar_fixture(Some(advertiser), "localhost").cell
    }

    struct ReadyHotRegistrarFixture {
        cell: SharedHotRegistrarCell,
        runtime: Arc<axon_sdk::invocation::LocalRuntime>,
        catalog: Arc<AxonAbilityCatalog>,
    }

    fn ready_hot_registrar_fixture(
        advertiser: Option<
            Arc<dyn crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiser>,
        >,
        authority_realm: &str,
    ) -> ReadyHotRegistrarFixture {
        let cell = SharedHotRegistrarCell::new();
        let dispatch_handle = Arc::new(std::sync::OnceLock::new());
        let registrar =
            crate::daemon::axon_bridge::hot_agent_registrar::HotAgentRegistrar::new_pending(
                Arc::new(Vec::new()),
                Arc::clone(&dispatch_handle),
                Arc::new(
                    crate::daemon::ability::builtins::agents::discover::DetachedDiscoverFederationResolver,
                ),
            );
        let runtime = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        );
        registrar
            .set_runtime(Arc::clone(&runtime))
            .expect("test runtime wired once");
        let authority_context = crate::daemon::ability::dispatch::AbilityAuthorityContext::for_device_authority_root_with_hosted_agents(
            crate::core::ura::device_ura(authority_realm, "dev-1"),
            Vec::<String>::new(),
        )
        .expect("test Device authority context");
        let catalog = Arc::new(AxonAbilityCatalog::new_with_runtime_and_authority_context(
            Arc::clone(&runtime),
            authority_context,
        ));
        dispatch_handle
            .set(Arc::clone(&catalog))
            .expect("test catalog wired once");
        if let Some(advertiser) = advertiser {
            registrar
                .set_hot_agent_advertiser(advertiser)
                .expect("test advertiser wired once");
        }
        assert!(
            cell.set(registrar).is_ok(),
            "test cell must accept its first registrar"
        );
        ReadyHotRegistrarFixture {
            cell,
            runtime,
            catalog,
        }
    }

    fn hosted_runtime_key(agent_ura: &str, registry_ability: &str) -> String {
        let public_name = crate::core::ura::owner_local_ability_name(agent_ura, registry_ability);
        crate::core::ura::owner_ability_ura(agent_ura, &public_name)
            .expect("hosted Agent runtime Ability URA")
    }

    #[test]
    fn registration_makes_lifecycle_abilities_dispatchable() {
        let mut reg = AxonAbilityCatalog::new_test_metadata_for_device_authority(TEST_DEVICE_URA);
        register(&mut reg, Arc::new(ready_hot_registrar()));
        assert!(reg.get_rpc(ABILITY_START_AGENT).is_some());
        assert!(reg.get_rpc(ABILITY_STOP_AGENT).is_some());
        assert!(reg.get_rpc(ABILITY_PURGE_AGENT).is_some());
        assert!(reg.get_rpc(ABILITY_REFRESH_AGENTS).is_some());
    }

    #[test]
    fn lifecycle_transaction_rejects_skipped_and_backward_transitions() {
        let mut start = AgentLifecycleTransaction::for_start(
            "agent.start",
            AgentRegistry::default(),
            local_agents::LocalAgentsFile::default(),
        );
        assert!(start.commit().is_err(), "start cannot commit from Prepared");
        start
            .transition(AgentLifecycleState::DurablePersisted)
            .unwrap();
        assert!(start.transition(AgentLifecycleState::Materialized).is_err());

        let mut stop = AgentLifecycleTransaction::for_stop(
            "agent.stop",
            AgentRegistry::default(),
            local_agents::LocalAgentsFile::default(),
        );
        assert!(stop
            .transition(AgentLifecycleState::DurablePersisted)
            .is_err());
        stop.transition(AgentLifecycleState::RuntimeSynchronized)
            .unwrap();
        stop.transition(AgentLifecycleState::DurablePersisted)
            .unwrap();
        stop.transition(AgentLifecycleState::IdentityPersisted)
            .unwrap();
        assert!(stop.commit().is_err(), "authority state is required");
    }

    #[test]
    fn stop_agent_rejects_device_sponsored_system_agent_ura() {
        with_isolated_home(|| {
            let err = agent_name_from_ura(
                "easynet:///r/localhost/agent/device.dev-1.terminal",
                "agent.stop",
            )
            .expect_err("System Agent URA must be refused on the lifecycle surface");
            let msg = err.to_string();
            assert!(
                msg.contains("RFC-005 §3.1.2"),
                "error cites the normative source: {msg}"
            );
            assert!(
                msg.contains("device-sponsored System Agent"),
                "error names the category error: {msg}"
            );
        });
    }

    #[test]
    fn start_agent_rejects_reserved_device_owner_name() {
        with_isolated_home(|| {
            for name in ["device.dev-1.sys", "device"] {
                let err = start_agent_handler(
                    json!({
                        "name": name,
                        "agent_type": "claude-code",
                    }),
                    &ready_hot_registrar(),
                )
                .expect_err("device-owned identity must be refused (DEC-F048)");
                let msg = err.to_string();
                assert!(
                    msg.contains("RFC-005 §3.1.2"),
                    "error cites the normative source: {msg}"
                );
                assert!(
                    msg.contains("device-sponsored"),
                    "error names the reserved-owner semantics: {msg}"
                );
                assert!(
                    !agents::load_agents()
                        .unwrap_or_default()
                        .agents
                        .contains_key(name),
                    "rejected name must not persist an agents.json row"
                );
            }
        });
    }

    #[test]
    fn start_agent_rejects_unjoined_hosted_agent_ura_derivation() {
        with_isolated_home(|| {
            let err = start_agent_handler(
                json!({
                    "name": "claude",
                    "agent_type": "claude-code",
                    "model": "sonnet",
                    "model_present": true,
                }),
                &ready_hot_registrar(),
            )
            .expect_err("unjoined daemon must not mint placeholder hosted-agent URAs");
            assert!(
                err.to_string().contains("requires joined credentials"),
                "error should surface credentials prerequisite: {err}"
            );
            assert!(
                !agents::load_agents()
                    .unwrap_or_default()
                    .agents
                    .contains_key("claude"),
                "unjoined failure must not persist a half-valid hosted agent row"
            );
        });
    }

    #[test]
    fn start_agent_rejects_model_without_explicit_model_present_intent() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let err = start_agent_handler(
                json!({
                    "name": "claude",
                    "agent_type": "claude-code",
                    "model": "sonnet",
                }),
                &ready_hot_registrar(),
            )
            .expect_err("model without explicit model_present must be rejected");
            assert!(
                err.to_string().contains("model_present"),
                "error should name required explicit model_present intent: {err}"
            );
            assert!(
                !agents::load_agents()
                    .unwrap_or_default()
                    .agents
                    .contains_key("claude"),
                "rejected ambiguous model write must not persist an agent row"
            );
        });
    }

    #[test]
    fn start_agent_materialize_syncs_hosted_ura_and_default_chat_manifest() {
        with_isolated_home(|| {
            seed_joined_credentials();

            let resp = start_agent_handler(
                json!({
                    "name": "anthropic",
                    "agent_type": "claude-code",
                    "model": "sonnet",
                    "model_present": true,
                    "materialize_directory": true,
                }),
                &ready_hot_registrar(),
            )
            .unwrap();

            let expected_ura = crate::core::ura::agent_ura("localhost", "user-dev", "anthropic");
            assert_eq!(resp["agent_ura"], json!(expected_ura));
            assert_eq!(
                local_agents::lookup_hosted_ura(&local_agents::load().unwrap(), "llm", "anthropic"),
                Some(expected_ura),
                "newly added agents must be visible to hosted-agent descriptor synthesis"
            );

            let registry = agents::load_agents().unwrap();
            let root = registry.agents["default/anthropic"]
                .root_path
                .clone()
                .unwrap();
            assert!(
                root.join("abilities").join("chat.ability.toml").exists(),
                "agent add must seed the default chat ability manifest"
            );
        });
    }

    #[test]
    fn start_agent_enrolls_authority_and_registers_runtime_without_restart() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let fixture = ready_hot_registrar_fixture(None, "localhost");
            let response = start_agent_handler(
                json!({
                    "name": "hot-worker",
                    "agent_type": "claude-code",
                }),
                &fixture.cell,
            )
            .expect("new hosted Agent must converge in one call");

            let agent_ura = response["agent_ura"].as_str().unwrap();
            assert_eq!(
                fixture
                    .catalog
                    .enrolled_hot_agent_authority_root("hot-worker")
                    .as_deref(),
                Some(agent_ura),
                "authority inventory must enroll the durable hosted-Agent root"
            );
            let chat_key = hosted_runtime_key(agent_ura, "hot-worker.chat");
            assert!(
                block_on_hot_registrar(fixture.runtime.has_ability(&chat_key)),
                "hot Agent chat row must be live before agent.start returns"
            );
        });
    }

    #[test]
    fn authority_inventory_rejects_unpersisted_agent_identity() {
        with_isolated_home(|| {
            let fixture = ready_hot_registrar_fixture(None, "localhost");
            let error = fixture
                .catalog
                .enroll_persisted_hot_agent_authority("forged")
                .expect_err("an arbitrary name cannot widen authority inventory");
            assert!(
                matches!(
                    error,
                    crate::daemon::ability::dispatch::HotAgentAuthorityInventoryError::DurableAgentMissing { .. }
                ),
                "unexpected enrollment error: {error}"
            );

            let mut registry = AgentRegistry::default();
            registry.agents.insert(
                "default/forged".to_string(),
                AgentEntry::new(AgentType::ClaudeCode, None),
            );
            agents::save_agents(&registry).unwrap();
            let error = fixture
                .catalog
                .enroll_persisted_hot_agent_authority("forged")
                .expect_err("durable row without hosted identity cannot enroll");
            assert!(
                matches!(
                    error,
                    crate::daemon::ability::dispatch::HotAgentAuthorityInventoryError::IdentityMissing { .. }
                ),
                "unexpected enrollment error: {error}"
            );
        });
    }

    #[test]
    fn start_agent_authority_failure_rolls_back_all_local_segments() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let fixture = ready_hot_registrar_fixture(None, "foreign-realm");
            let root = config::agents_root().join("rollback-worker");
            let error = start_agent_handler(
                json!({
                    "name": "rollback-worker",
                    "agent_type": "claude-code",
                    "materialize_directory": true,
                }),
                &fixture.cell,
            )
            .expect_err("foreign authority inventory must reject enrollment");
            assert!(error.to_string().contains("rollback=completed"), "{error}");
            assert!(!agents::load_agents()
                .unwrap()
                .agents
                .contains_key("default/rollback-worker"));
            assert_eq!(
                local_agents::lookup_hosted_ura(
                    &local_agents::load().unwrap(),
                    "llm",
                    "rollback-worker"
                ),
                None
            );
            assert!(
                !root.exists(),
                "created Agent directory must be compensated"
            );
            assert_eq!(
                fixture
                    .catalog
                    .enrolled_hot_agent_authority_root("rollback-worker"),
                None
            );
            let agent_ura = crate::core::ura::agent_ura("localhost", "user-dev", "rollback-worker");
            let chat_key = hosted_runtime_key(&agent_ura, "rollback-worker.chat");
            assert!(!block_on_hot_registrar(
                fixture.runtime.has_ability(&chat_key)
            ));
        });
    }

    #[test]
    fn stop_agent_revokes_authority_and_runtime_rows() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let fixture = ready_hot_registrar_fixture(None, "localhost");
            let response = start_agent_handler(
                json!({
                    "name": "ephemeral",
                    "agent_type": "claude-code",
                }),
                &fixture.cell,
            )
            .unwrap();
            let agent_ura = response["agent_ura"].as_str().unwrap().to_string();
            let chat_key = hosted_runtime_key(&agent_ura, "ephemeral.chat");
            assert!(block_on_hot_registrar(
                fixture.runtime.has_ability(&chat_key)
            ));
            assert!(fixture
                .catalog
                .enrolled_hot_agent_authority_root("ephemeral")
                .is_some());

            let response = stop_agent_handler(json!({"name": "ephemeral"}), &fixture.cell).unwrap();
            assert_eq!(response["ack"], true);
            assert!(!block_on_hot_registrar(
                fixture.runtime.has_ability(&chat_key)
            ));
            assert_eq!(
                fixture
                    .catalog
                    .enrolled_hot_agent_authority_root("ephemeral"),
                None,
                "stop commit must revoke the catalog-owned authority root"
            );
        });
    }

    #[tokio::test]
    async fn start_agent_hot_advertises_joined_hosted_ura_when_bridge_is_wired() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        seed_joined_credentials();
        let advertiser = Arc::new(RecordingHotAdvertiser::default());
        let hot_registrar = hot_registrar_with_advertiser(Arc::clone(&advertiser));

        let resp = start_agent_handler(
            json!({
                "name": "anthropic",
                "agent_type": "claude-code",
                "materialize_directory": true,
            }),
            &hot_registrar,
        )
        .unwrap();

        let expected_ura = crate::core::ura::agent_ura("localhost", "user-dev", "anthropic");
        assert_eq!(resp["hub_advertised"], false);
        assert_eq!(resp["hub_advertise_state"], "scheduled");
        assert_eq!(resp["hub_advertise_error"], Value::Null);
        for _ in 0..50 {
            if !advertiser.requests.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            advertiser.requests.lock().unwrap().as_slice(),
            [expected_ura.as_str()],
            "hot-added agent must be advertised by the session publication worker"
        );
    }

    #[test]
    fn start_agent_preserves_existing_row_and_signals_replaced_prior() {
        // Phase 5c semantic: a second `agent.start` call on
        // an existing row LEAVES the stored entry alone (so the CLI
        // `agent add` flow's rich v2 row survives the daemon's
        // notify-back hop) but still flags `replaced_prior=true`
        // so the operator can see the row existed.
        with_isolated_home(|| {
            seed_joined_credentials();
            start_agent_handler(
                json!({
                    "name": "claude",
                    "agent_type": "claude-code",
                }),
                &ready_hot_registrar(),
            )
            .unwrap();
            let resp = start_agent_handler(
                json!({
                    "name": "claude",
                    "agent_type": "codex",
                }),
                &ready_hot_registrar(),
            )
            .unwrap();
            assert_eq!(
                resp["replaced_prior"], true,
                "second insertion of same name MUST flag replaced_prior=true"
            );
            // Preserve invariant: agent_type stays `claude-code`
            // even though the second call asked for `codex`. The
            // stored row is authoritative; the daemon-side notify
            // is runtime-registration-only.
            let registry = agents::load_agents().unwrap();
            assert_eq!(
                registry.agents.get("default/claude").unwrap().agent_type,
                AgentType::ClaudeCode,
                "second start of same name MUST NOT overwrite the stored row's agent_type"
            );
        });
    }

    #[test]
    fn start_agent_rejects_missing_name() {
        let err = start_agent_handler(json!({"agent_type": "claude-code"}), &ready_hot_registrar())
            .unwrap_err();
        assert!(format!("{err}").contains("name"));
    }

    #[test]
    fn start_agent_persists_external_command_and_args() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let resp = start_agent_handler(
                json!({
                    "name": "semop",
                    "agent_type": "external",
                    "command": "/bin/cat",
                    "command_args": ["--number"],
                    "materialize_directory": true,
                }),
                &ready_hot_registrar(),
            )
            .unwrap();
            assert_eq!(resp["agent_type"], "external");
            let registry = agents::load_agents().unwrap();
            let stored = registry.agents.get("default/semop").unwrap();
            assert_eq!(stored.agent_type, AgentType::External);
            assert_eq!(stored.command, "/bin/cat");
            assert_eq!(stored.args, vec!["--number".to_string()]);
        });
    }

    #[test]
    fn start_agent_rejects_external_without_command() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let err = start_agent_handler(
                json!({
                    "name": "semop",
                    "agent_type": "external",
                    "materialize_directory": true,
                }),
                &ready_hot_registrar(),
            )
            .unwrap_err();
            assert!(format!("{err}").contains("external agents require `command`"));
        });
    }

    #[test]
    fn start_agent_accepts_rich_entry_without_top_level_agent_type() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let resp = start_agent_handler(
                json!({
                    "name": "codex-rich",
                    "entry": {
                        "agent_type": "codex",
                        "model": "gpt-5",
                        "label": "Codex rich row"
                    }
                }),
                &ready_hot_registrar(),
            )
            .unwrap();
            assert_eq!(resp["agent_type"], "codex");
            assert_eq!(resp["model"], "gpt-5");

            let registry = agents::load_agents().unwrap();
            let stored = registry.agents.get("default/codex-rich").unwrap();
            assert_eq!(stored.agent_type, AgentType::Codex);
            assert_eq!(stored.model.as_deref(), Some("gpt-5"));
        });
    }

    #[test]
    fn start_agent_rejects_conflicting_top_level_and_entry_agent_type() {
        let err = start_agent_handler(
            json!({
                "name": "conflict",
                "agent_type": "claude-code",
                "entry": {
                    "agent_type": "codex"
                }
            }),
            &ready_hot_registrar(),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("does not match"));
    }

    #[test]
    fn start_agent_rejects_missing_agent_type_when_entry_absent() {
        let err = start_agent_handler(json!({"name": "x"}), &ready_hot_registrar()).unwrap_err();
        assert!(format!("{err}").contains("agent_type"));
    }

    #[test]
    fn start_agent_materialize_reuses_existing_root_path_when_root_omitted() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let custom_root = crate::daemon::persistence::config::home_dir()
                .join("project")
                .join("agents")
                .join("claude");
            start_agent_handler(
                json!({
                    "name": "claude",
                    "agent_type": "claude-code",
                    "model": "sonnet",
                    "model_present": true,
                    "root_path": custom_root,
                    "materialize_directory": true,
                }),
                &ready_hot_registrar(),
            )
            .unwrap();

            let resp = start_agent_handler(
                json!({
                    "name": "claude",
                    "agent_type": "claude-code",
                    "model": "opus",
                    "model_present": true,
                    "materialize_directory": true,
                    "update_existing_spec": true,
                }),
                &ready_hot_registrar(),
            )
            .unwrap();

            let stored_root = agents::load_agents().unwrap().agents["default/claude"]
                .root_path
                .clone()
                .unwrap();
            assert_eq!(
                resp["root_path"],
                json!(stored_root.to_string_lossy().to_string())
            );
            assert!(
                stored_root.ends_with("project/agents/claude"),
                "must preserve project-local root, got {}",
                stored_root.display()
            );
            let spec = AgentSpec::from_toml_str(
                &std::fs::read_to_string(stored_root.join("agent.toml")).unwrap(),
            )
            .unwrap();
            assert_eq!(spec.model.as_deref(), Some("opus"));
        });
    }

    #[test]
    fn start_agent_rejects_unknown_agent_type() {
        let err = start_agent_handler(
            json!({
                "name": "x",
                "agent_type": "totally-not-a-runtime",
            }),
            &ready_hot_registrar(),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("unknown agent type"));
    }

    #[test]
    fn stop_agent_by_name_acks_true_and_removes_row() {
        with_isolated_home(|| {
            seed_joined_credentials();
            start_agent_handler(
                json!({
                    "name": "claude",
                    "agent_type": "claude-code",
                }),
                &ready_hot_registrar(),
            )
            .unwrap();

            let resp =
                stop_agent_handler(json!({"name": "claude"}), &ready_hot_registrar()).unwrap();
            assert_eq!(resp["ack"], true);
            assert!(!agents::load_agents().unwrap().agents.contains_key("claude"));
            assert_eq!(
                local_agents::lookup_hosted_ura(&local_agents::load().unwrap(), "llm", "claude"),
                None,
                "stopping an agent must remove its hosted llm mapping"
            );
        });
    }

    #[test]
    fn purge_agent_deletes_only_the_registered_agent_root() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let registrar = ready_hot_registrar();
            let response = start_agent_handler(
                json!({
                    "name": "claude",
                    "agent_type": "claude-code",
                    "materialize_directory": true,
                }),
                &registrar,
            )
            .unwrap();
            let root = std::path::PathBuf::from(response["root_path"].as_str().unwrap());
            assert!(root.join("agent.toml").exists());

            let response = purge_agent_handler(json!({"name": "claude"}), &registrar).unwrap();

            assert_eq!(response["ack"], true);
            assert_eq!(response["purge_state"], "purged");
            assert_eq!(response["purged_path"], root.to_string_lossy().as_ref());
            assert!(!root.exists());
            assert!(!agents::load_agents().unwrap().agents.contains_key("claude"));
        });
    }

    #[cfg(not(unix))]
    #[test]
    fn unsupported_platform_rejects_purge_before_journal_or_registry_mutation() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let registrar = ready_hot_registrar();
            let response = start_agent_handler(
                json!({
                    "name": "platform-gate",
                    "agent_type": "claude-code",
                    "materialize_directory": true,
                }),
                &registrar,
            )
            .unwrap();
            let root = std::path::PathBuf::from(response["root_path"].as_str().unwrap());

            let error = purge_agent_handler(json!({"name": "platform-gate"}), &registrar)
                .expect_err("unsupported targets must reject before quarantine");
            assert!(error.downcast_ref::<PurgePlatformUnsupported>().is_some());
            assert!(root.exists());
            assert!(agents::load_agents()
                .unwrap()
                .agents
                .contains_key("platform-gate"));
            assert!(lifecycle_store::load_purge_journal().unwrap().is_none());
        });
    }

    #[test]
    fn restart_replays_identical_tombstone_after_publication_window_crash() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let advertiser = Arc::new(RecordingHotAdvertiser::default());
            let first = hot_registrar_with_advertiser(Arc::clone(&advertiser));
            let response = start_agent_handler(
                json!({
                    "name": "restartable",
                    "agent_type": "claude-code",
                    "materialize_directory": true,
                }),
                &first,
            )
            .unwrap();
            let root = std::path::PathBuf::from(response["root_path"].as_str().unwrap());
            let identity_advertisements = advertiser.requests.lock().unwrap().len();

            PURGE_AFTER_TOMBSTONE_PUBLISH_CRASH.with(|slot| slot.set(true));
            let error = purge_agent_handler(json!({"name": "restartable"}), &first)
                .expect_err("publication-window crash must leave committed journal");
            PURGE_AFTER_TOMBSTONE_PUBLISH_CRASH.with(|slot| slot.set(false));
            assert!(error
                .to_string()
                .contains("after Hub tombstone publication"));

            assert!(lifecycle_store::load_purge_journal().unwrap().is_none());
            let outbox = lifecycle_store::load_publication_outbox().unwrap();
            let pending = outbox
                .entries
                .first()
                .expect("publication crash leaves a durable outbox row")
                .clone();
            assert_eq!(pending.stage, AgentPurgePublicationStage::TombstonePending);
            let publication = &pending.publication;
            let cursor_file = crate::daemon::persistence::owner_projections::load().unwrap();
            let cursor = cursor_file.cursor_for(&pending.agent_ura).unwrap();
            assert_eq!(cursor.projection_revision, publication.projection_revision);
            assert_eq!(cursor.projection_digest, publication.projection_digest);
            assert!(!root.exists());
            drop(outbox);
            expire_publication_claim(&pending.transaction_id);

            drop(first);
            let advertiser_trait: Arc<
                dyn crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiser,
            > = advertiser.clone();
            let restarted = ready_hot_registrar_fixture(Some(advertiser_trait), "localhost");
            assert!(recover_pending_purge_on_boot(&restarted.cell).unwrap());

            assert!(lifecycle_store::load_purge_journal().unwrap().is_none());
            assert!(lifecycle_store::load_publication_outbox()
                .unwrap()
                .entries
                .is_empty());
            let cursor_file = crate::daemon::persistence::owner_projections::load().unwrap();
            let retired = cursor_file.cursor_for(&pending.agent_ura).unwrap();
            assert_eq!(
                retired.lifecycle,
                crate::daemon::persistence::owner_projections::OwnerProjectionCursorLifecycle::Retired
            );
            assert!(cursor_file.active_cursor_for(&pending.agent_ura).is_none());
            let payloads = advertiser.ability_payloads.lock().unwrap();
            assert!(payloads.len() >= 3, "start + two tombstone publications");
            let first: serde_json::Value =
                serde_json::from_slice(&payloads[payloads.len() - 2]).unwrap();
            let replay: serde_json::Value =
                serde_json::from_slice(&payloads[payloads.len() - 1]).unwrap();
            assert_eq!(first["projection_revision"], replay["projection_revision"]);
            assert_eq!(first["projection_digest"], replay["projection_digest"]);
            assert!(
                replay["purge_delivery"]["delivery_fence"].as_u64().unwrap()
                    > first["purge_delivery"]["delivery_fence"].as_u64().unwrap(),
                "recovery keeps projection identity but advances the transport fence"
            );
            assert_eq!(
                advertiser
                    .idempotent_projection_replays
                    .load(std::sync::atomic::Ordering::SeqCst),
                1,
                "the Hub revision fence must classify exact replay as idempotent"
            );
            assert_eq!(advertiser.revokes.lock().unwrap().len(), 1);
            assert_eq!(
                advertiser.requests.lock().unwrap().len(),
                identity_advertisements,
                "purge tombstone replay must use projection-only publication"
            );
        });
    }

    #[test]
    fn restart_replays_revoke_with_the_same_purge_transaction_after_success_window_crash() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let advertiser = Arc::new(RecordingHotAdvertiser::default());
            let fixture = hot_registrar_with_advertiser(Arc::clone(&advertiser));
            start_agent_handler(
                json!({
                    "name": "revoke-crash",
                    "agent_type": "claude-code",
                    "materialize_directory": true,
                }),
                &fixture,
            )
            .unwrap();

            PURGE_AFTER_REVOKE_PUBLISH_CRASH.with(|slot| slot.set(true));
            let error = purge_agent_handler(json!({"name": "revoke-crash"}), &fixture)
                .expect_err("remote revoke success before local deletion is crash-replayable");
            PURGE_AFTER_REVOKE_PUBLISH_CRASH.with(|slot| slot.set(false));
            assert!(error.to_string().contains("after Hub revoke publication"));

            let outbox = lifecycle_store::load_publication_outbox().unwrap();
            let pending = outbox.entries.first().unwrap().clone();
            assert_eq!(pending.stage, AgentPurgePublicationStage::RevokePending);
            assert_eq!(
                advertiser.revoke_transactions.lock().unwrap().as_slice(),
                &[Some(pending.transaction_id.clone())]
            );
            drop(outbox);
            expire_publication_claim(&pending.transaction_id);

            assert!(recover_pending_purge_on_boot(&fixture).unwrap());
            assert_eq!(
                advertiser.revoke_transactions.lock().unwrap().as_slice(),
                &[
                    Some(pending.transaction_id.clone()),
                    Some(pending.transaction_id.clone())
                ],
                "replay carries the same Hub deduplication key"
            );
            assert!(lifecycle_store::load_publication_outbox()
                .unwrap()
                .entries
                .is_empty());
        });
    }

    #[test]
    fn unavailable_publisher_leaves_only_a_durable_outbox_without_quarantine_deadlock() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let fixture = ready_hot_registrar_fixture(None, "localhost");
            let response = start_agent_handler(
                json!({
                    "name": "publisher-wait",
                    "agent_type": "claude-code",
                    "materialize_directory": true,
                }),
                &fixture.cell,
            )
            .unwrap();
            let root = std::path::PathBuf::from(response["root_path"].as_str().unwrap());

            let response =
                purge_agent_handler(json!({"name": "publisher-wait"}), &fixture.cell).unwrap();
            assert_eq!(response["purge_state"], "purged");
            assert_eq!(response["publication_state"], "pending");
            assert!(!root.exists());
            assert!(lifecycle_store::load_purge_journal().unwrap().is_none());
            let outbox = lifecycle_store::load_publication_outbox().unwrap();
            assert_eq!(outbox.entries.len(), 1);
            assert_eq!(
                outbox.entries[0].stage,
                AgentPurgePublicationStage::TombstonePending
            );

            assert!(!recover_pending_purge_on_boot(&fixture.cell).unwrap());
            assert!(lifecycle_store::load_purge_journal().unwrap().is_none());
            let reuse_error = start_agent_handler(
                json!({"name": "publisher-wait", "agent_type": "codex"}),
                &fixture.cell,
            )
            .expect_err("the same logical identity must wait for ordered publication");
            assert!(reuse_error.to_string().contains("identity reuse is fenced"));

            let second = start_agent_handler(
                json!({"name": "independent", "agent_type": "codex", "materialize_directory": true}),
                &fixture.cell,
            )
            .expect("an unrelated lifecycle mutation must not be blocked by publication retry");
            let second_root = std::path::PathBuf::from(second["root_path"].as_str().unwrap());
            let second_purge =
                purge_agent_handler(json!({"name": "independent"}), &fixture.cell).unwrap();
            assert_eq!(second_purge["purge_state"], "purged");
            assert!(!second_root.exists());
        });
    }

    #[test]
    fn backed_off_revoke_poison_does_not_block_later_purge_publication() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let advertiser = Arc::new(SelectiveFailureHotAdvertiser::default());
            let advertiser_trait: Arc<
                dyn crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiser,
            > = advertiser.clone();
            let fixture = ready_hot_registrar_fixture(Some(advertiser_trait), "localhost");
            for name in ["poison-first", "healthy-later"] {
                start_agent_handler(
                    json!({
                        "name": name,
                        "agent_type": "claude-code",
                        "materialize_directory": true,
                    }),
                    &fixture.cell,
                )
                .unwrap();
            }
            let identities = local_agents::load().unwrap();
            let poison_ura = hosted_agent_ura_from_file(&identities, "poison-first").unwrap();
            let healthy_ura = hosted_agent_ura_from_file(&identities, "healthy-later").unwrap();
            advertiser.fail_revoke_for(&poison_ura);

            let poison = purge_agent_handler(json!({"name": "poison-first"}), &fixture.cell)
                .expect("local purge commits before publication");
            assert_eq!(poison["publication_state"], "pending");
            assert!(poison["publication_error"]
                .as_str()
                .unwrap()
                .contains("poisoned revoke"));
            let healthy = purge_agent_handler(json!({"name": "healthy-later"}), &fixture.cell)
                .expect("later transaction must publish independently");
            assert_eq!(healthy["publication_state"], "published");

            let outbox = lifecycle_store::load_publication_outbox().unwrap();
            assert_eq!(outbox.entries.len(), 1);
            let poison = &outbox.entries[0];
            assert_eq!(poison.agent_ura, poison_ura);
            assert_eq!(poison.retry.attempts, 1);
            let evidence = poison.retry.last_failure.as_ref().unwrap();
            assert_eq!(evidence.stage, AgentPurgePublicationStage::RevokePending);
            assert!(evidence.error.contains("poisoned revoke"));
            assert_eq!(
                advertiser.recording.revokes.lock().unwrap().as_slice(),
                &[healthy_ura],
                "only the independently healthy transaction reaches revoke"
            );
        });
    }

    #[test]
    fn restart_redrive_isolates_poisoned_transactions_and_preserves_retry_evidence() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let advertiser = Arc::new(SelectiveFailureHotAdvertiser::default());
            let advertiser_trait: Arc<
                dyn crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiser,
            > = advertiser.clone();
            let first_runtime = ready_hot_registrar_fixture(Some(advertiser_trait), "localhost");
            for name in ["restart-poison", "restart-healthy"] {
                start_agent_handler(
                    json!({
                        "name": name,
                        "agent_type": "claude-code",
                        "materialize_directory": true,
                    }),
                    &first_runtime.cell,
                )
                .unwrap();
            }
            let identities = local_agents::load().unwrap();
            let poison_ura = hosted_agent_ura_from_file(&identities, "restart-poison").unwrap();
            let healthy_ura = hosted_agent_ura_from_file(&identities, "restart-healthy").unwrap();
            advertiser.fail_tombstone_for(&poison_ura);
            advertiser.fail_tombstone_for(&healthy_ura);

            purge_agent_handler(json!({"name": "restart-poison"}), &first_runtime.cell).unwrap();
            purge_agent_handler(json!({"name": "restart-healthy"}), &first_runtime.cell).unwrap();
            assert_eq!(
                lifecycle_store::load_publication_outbox()
                    .unwrap()
                    .entries
                    .len(),
                2
            );

            advertiser.allow_tombstone_for(&healthy_ura);
            drop(first_runtime);
            let advertiser_trait: Arc<
                dyn crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiser,
            > = advertiser.clone();
            let restarted = ready_hot_registrar_fixture(Some(advertiser_trait), "localhost");
            assert!(
                !recover_pending_purge_on_boot(&restarted.cell).unwrap(),
                "the poisoned transaction remains durable after restart"
            );

            let outbox = lifecycle_store::load_publication_outbox().unwrap();
            assert_eq!(outbox.entries.len(), 1);
            let poison = &outbox.entries[0];
            assert_eq!(poison.agent_ura, poison_ura);
            assert_eq!(poison.retry.attempts, 2);
            let evidence = poison.retry.last_failure.as_ref().unwrap();
            assert_eq!(evidence.attempt, 2);
            assert!(evidence.error.contains("poisoned tombstone"));
            assert!(advertiser
                .recording
                .revokes
                .lock()
                .unwrap()
                .contains(&healthy_ura));

            advertiser.allow_tombstone_for(&poison_ura);
            assert!(recover_pending_purge_on_boot(&restarted.cell).unwrap());
            assert!(lifecycle_store::load_publication_outbox()
                .unwrap()
                .entries
                .is_empty());
        });
    }

    #[test]
    fn exhausted_publication_is_not_automatically_retried_and_retains_identity_fence() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let advertiser = Arc::new(SelectiveFailureHotAdvertiser::default());
            let advertiser_trait: Arc<
                dyn crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiser,
            > = advertiser.clone();
            let fixture = ready_hot_registrar_fixture(Some(advertiser_trait), "localhost");
            start_agent_handler(
                json!({
                    "name": "dead-letter",
                    "agent_type": "claude-code",
                    "materialize_directory": true,
                }),
                &fixture.cell,
            )
            .unwrap();
            let identity =
                hosted_agent_ura_from_file(&local_agents::load().unwrap(), "dead-letter").unwrap();
            advertiser.fail_tombstone_for(&identity);

            purge_agent_handler(json!({"name": "dead-letter"}), &fixture.cell).unwrap();
            for _ in 1..lifecycle_store::PUBLICATION_MAX_ATTEMPTS_PER_STAGE {
                assert!(!recover_pending_purge_on_boot(&fixture.cell).unwrap());
            }

            let outbox = lifecycle_store::load_publication_outbox().unwrap();
            let dead_letter = outbox.entries.first().unwrap();
            let transaction_id = dead_letter.transaction_id.clone();
            assert!(matches!(
                dead_letter.retry.state,
                lifecycle_store::AgentPurgePublicationRetryState::ReconciliationRequired { .. }
            ));
            assert_eq!(
                dead_letter.retry.attempts,
                lifecycle_store::PUBLICATION_MAX_ATTEMPTS_PER_STAGE
            );
            assert_eq!(dead_letter.agent_ura, identity);
            let calls_at_dead_letter = advertiser.recording.ability_payloads.lock().unwrap().len();
            drop(outbox);

            assert!(matches!(
                publication_recovery_status(&fixture.cell, PurgePublicationRetryTrigger::Scheduled)
                    .unwrap(),
                PurgeRecoveryStatus::PublicationPending(_)
            ));
            assert!(!recover_pending_purge_on_boot(&fixture.cell).unwrap());
            assert_eq!(
                advertiser.recording.ability_payloads.lock().unwrap().len(),
                calls_at_dead_letter,
                "connectivity-ready drains must not retry reconciliation-required work"
            );
            let reuse_error = start_agent_handler(
                json!({"name": "dead-letter", "agent_type": "codex"}),
                &fixture.cell,
            )
            .expect_err("dead-lettered publication retains the logical identity fence");
            assert!(reuse_error.to_string().contains("identity reuse is fenced"));

            let command = lifecycle_store::AgentPurgeReconciliationCommand {
                command_id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                transaction_id: transaction_id.clone(),
                actor_ura: "easynet:///r/test/device/dev-1".to_string(),
                action: lifecycle_store::AgentPurgePublicationReconciliation::Retry,
            };
            let authorization = lifecycle_store::AuthorizedPurgeReconciliation::from_admission(
                command.actor_ura.clone(),
                "test-authority",
            )
            .unwrap();
            let retried =
                lifecycle_store::reconcile_publication(&command, &authorization, 1_000_000)
                    .unwrap();
            assert_eq!(
                retried.entry.retry.state,
                lifecycle_store::AgentPurgePublicationRetryState::Ready
            );
            assert!(retried.entry.retry.last_reconciliation.is_some());
            assert!(!retried.replayed);
            let replay =
                lifecycle_store::reconcile_publication(&command, &authorization, 1_000_001)
                    .unwrap();
            assert!(replay.replayed);
            let audited = lifecycle_store::load_publication_outbox().unwrap();
            assert_eq!(audited.reconciliation_audit.len(), 1);
            assert_eq!(
                audited.reconciliation_audit[0].command.command_id,
                command.command_id
            );
            drop(audited);
            advertiser.allow_tombstone_for(&identity);
            assert!(recover_pending_purge_on_boot(&fixture.cell).unwrap());
            assert!(lifecycle_store::load_publication_outbox()
                .unwrap()
                .entries
                .is_empty());
        });
    }

    #[test]
    fn concurrent_drains_do_not_duplicate_a_live_publication_claim() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let (started_tx, started_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            let advertiser = Arc::new(BlockingHotAdvertiser {
                recording: RecordingHotAdvertiser::default(),
                fail_tombstone: std::sync::atomic::AtomicBool::new(false),
                block_next_tombstone: std::sync::atomic::AtomicBool::new(false),
                started: started_tx,
                release: std::sync::Mutex::new(release_rx),
            });
            let advertiser_trait: Arc<
                dyn crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiser,
            > = advertiser.clone();
            let fixture = Arc::new(ready_hot_registrar_fixture(
                Some(advertiser_trait),
                "localhost",
            ));
            start_agent_handler(
                json!({
                    "name": "concurrent-drain",
                    "agent_type": "claude-code",
                    "materialize_directory": true,
                }),
                &fixture.cell,
            )
            .unwrap();
            advertiser
                .fail_tombstone
                .store(true, std::sync::atomic::Ordering::SeqCst);
            purge_agent_handler(json!({"name": "concurrent-drain"}), &fixture.cell).unwrap();
            advertiser
                .fail_tombstone
                .store(false, std::sync::atomic::Ordering::SeqCst);
            advertiser
                .block_next_tombstone
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let published_before_race = advertiser.recording.ability_payloads.lock().unwrap().len();

            let first_fixture = Arc::clone(&fixture);
            let first =
                std::thread::spawn(move || recover_pending_purge_on_boot(&first_fixture.cell));
            started_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("first drain reaches the provider");

            assert!(
                !recover_pending_purge_on_boot(&fixture.cell).unwrap(),
                "second drain observes the durable live claim"
            );
            assert_eq!(
                advertiser.recording.ability_payloads.lock().unwrap().len(),
                published_before_race,
                "the second drain must not duplicate the in-flight tombstone"
            );

            release_tx.send(()).unwrap();
            assert!(first.join().unwrap().unwrap());
            assert!(lifecycle_store::load_publication_outbox()
                .unwrap()
                .entries
                .is_empty());
            assert_eq!(
                advertiser.recording.ability_payloads.lock().unwrap().len(),
                published_before_race + 1,
                "exactly one tombstone is published by the racing drains"
            );
        });
    }

    #[test]
    fn no_active_projection_is_persisted_as_explicit_no_publication_fact() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let fixture = ready_hot_registrar_fixture(None, "localhost");
            start_agent_handler(
                json!({
                    "name": "never-published",
                    "agent_type": "claude-code",
                    "materialize_directory": true,
                }),
                &fixture.cell,
            )
            .unwrap();
            crate::daemon::persistence::owner_projections::replace(
                &crate::daemon::persistence::owner_projections::OwnerProjectionCursorFile::default(
                ),
            )
            .unwrap();
            PURGE_CRASH_STAGE.with(|slot| slot.set(Some(AgentPurgeStage::TombstonePrepared)));
            let error = purge_agent_handler(json!({"name": "never-published"}), &fixture.cell)
                .expect_err("failpoint preserves the no-publication decision");
            PURGE_CRASH_STAGE.with(|slot| slot.set(None));
            assert!(error.to_string().contains("injected Agent purge crash"));

            let journal = lifecycle_store::load_purge_journal().unwrap().unwrap();
            assert_eq!(journal.stage, AgentPurgeStage::TombstonePrepared);
            assert_eq!(
                journal.publication_plan,
                AgentPurgePublicationPlan::NotRequired {
                    reason: AgentPurgeNoPublicationReason::NoActiveOwnerProjection,
                }
            );
            assert!(
                recover_pending_purge_on_boot(&fixture.cell).unwrap(),
                "an explicit no-publication fact does not require a publisher"
            );
        });
    }

    #[test]
    fn corrupt_credentials_do_not_block_local_purge_or_publication_replay() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let advertiser = Arc::new(RecordingHotAdvertiser::default());
            let fixture = hot_registrar_with_advertiser(Arc::clone(&advertiser));
            start_agent_handler(
                json!({
                    "name": "credential-wait",
                    "agent_type": "claude-code",
                    "materialize_directory": true,
                }),
                &fixture,
            )
            .unwrap();
            let initial_payloads = advertiser.ability_payloads.lock().unwrap().len();
            std::fs::write(config::state_dir().join("credentials.json"), b"{")
                .expect("corrupt credentials for recovery test");

            let response = purge_agent_handler(json!({"name": "credential-wait"}), &fixture)
                .expect("cursor-owned host identity makes purge independent of credentials");
            assert_eq!(response["purge_state"], "purged");
            assert_eq!(response["publication_state"], "published");
            assert!(lifecycle_store::load_purge_journal().unwrap().is_none());
            assert!(lifecycle_store::load_publication_outbox()
                .unwrap()
                .entries
                .is_empty());
            assert!(advertiser.ability_payloads.lock().unwrap().len() > initial_payloads);
            assert_eq!(advertiser.revokes.lock().unwrap().len(), 1);
        });
    }

    #[test]
    fn pre_replay_recovery_finishes_committed_quarantine_with_corrupt_credentials() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let fixture = ready_hot_registrar_fixture(None, "localhost");
            let response = start_agent_handler(
                json!({
                    "name": "boot-credential-independent",
                    "agent_type": "claude-code",
                    "materialize_directory": true,
                }),
                &fixture.cell,
            )
            .unwrap();
            let root = std::path::PathBuf::from(response["root_path"].as_str().unwrap());

            PURGE_CRASH_STAGE.with(|slot| slot.set(Some(AgentPurgeStage::Committed)));
            purge_agent_handler(
                json!({"name": "boot-credential-independent"}),
                &fixture.cell,
            )
            .expect_err("commit failpoint simulates process loss before local finalization");
            PURGE_CRASH_STAGE.with(|slot| slot.set(None));
            let journal = lifecycle_store::load_purge_journal().unwrap().unwrap();
            assert!(journal.quarantine_path.exists());
            std::fs::write(config::state_dir().join("credentials.json"), b"{")
                .expect("corrupt credentials before pre-replay recovery");

            assert!(recover_pending_purge_before_agent_replay(&fixture.cell).unwrap());
            assert!(!root.exists());
            assert!(!journal.quarantine_path.exists());
            assert!(lifecycle_store::load_purge_journal().unwrap().is_none());
            assert_eq!(
                lifecycle_store::load_publication_outbox()
                    .unwrap()
                    .entries
                    .len(),
                1
            );
        });
    }

    #[cfg(feature = "axon-pb")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn outbox_ready_hook_redrives_publisher_waiting_purge() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        seed_joined_credentials();
        let fixture = ready_hot_registrar_fixture(None, "localhost");
        start_agent_handler(
            json!({
                "name": "outbox-wait",
                "agent_type": "claude-code",
                "materialize_directory": true,
            }),
            &fixture.cell,
        )
        .unwrap();
        let response = purge_agent_handler(json!({"name": "outbox-wait"}), &fixture.cell)
            .expect("local purge completes while publication waits");
        assert_eq!(response["publication_state"], "pending");

        let advertiser: Arc<
            dyn crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiser,
        > = Arc::new(RecordingHotAdvertiser::default());
        fixture
            .cell
            .get()
            .unwrap()
            .set_hot_agent_advertiser(advertiser)
            .unwrap();
        let cell = Arc::new(fixture.cell);
        let outbox =
            crate::daemon::invocation::bidi::session_escalation::SharedSessionOutbox::new();
        crate::daemon::boot::invocation::register_purge_recovery_on_outbox_ready(
            &outbox,
            Arc::clone(&cell),
        );
        assert!(lifecycle_store::load_purge_journal().unwrap().is_none());
        assert!(!lifecycle_store::load_publication_outbox()
            .unwrap()
            .entries
            .is_empty());
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        outbox.set(crate::daemon::invocation::bidi::session_initiator::SessionUpSender::new(tx));

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
        while !lifecycle_store::load_publication_outbox()
            .unwrap()
            .entries
            .is_empty()
            && tokio::time::Instant::now() < deadline
        {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(
            lifecycle_store::load_publication_outbox()
                .unwrap()
                .entries
                .is_empty(),
            "session-ready hook must drain the durable publication outbox"
        );
    }

    const PURGE_CRASH_CHILD_ENV: &str = "EASYNET_PURGE_REAL_CRASH_CHILD";
    const PURGE_CRASH_CHILD_NAME_ENV: &str = "EASYNET_PURGE_REAL_CRASH_NAME";

    #[test]
    fn purge_real_crash_child_process() {
        if std::env::var_os(PURGE_CRASH_CHILD_ENV).is_none() {
            return;
        }
        let name = std::env::var(PURGE_CRASH_CHILD_NAME_ENV).unwrap();
        let registrar = ready_hot_registrar();
        refresh_agents_handler(json!({"name": name}), &registrar).unwrap();
        PURGE_CRASH_STAGE.with(|slot| slot.set(Some(AgentPurgeStage::Committed)));
        let error = purge_agent_handler(json!({"name": name}), &registrar)
            .expect_err("child failpoint must persist Committed before abort");
        assert!(error.to_string().contains("injected Agent purge crash"));
        std::process::abort();
    }

    #[test]
    fn real_process_crash_and_restart_rolls_committed_purge_forward() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let name = "process-crash";
            let response = start_agent_handler(
                json!({
                    "name": name,
                    "agent_type": "claude-code",
                    "materialize_directory": true,
                }),
                &ready_hot_registrar(),
            )
            .unwrap();
            let root = std::path::PathBuf::from(response["root_path"].as_str().unwrap());
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .arg("--exact")
                .arg("daemon::ability::builtins::agents::lifecycle::tests::purge_real_crash_child_process")
                .arg("--nocapture")
                .env(PURGE_CRASH_CHILD_ENV, "1")
                .env(PURGE_CRASH_CHILD_NAME_ENV, name)
                .env("HOME", config::home_dir())
                .status()
                .expect("spawn real purge crash child");
            assert!(!status.success(), "child must terminate abnormally");

            let journal = lifecycle_store::load_purge_journal().unwrap().unwrap();
            assert_eq!(journal.stage, AgentPurgeStage::Committed);
            assert!(!root.exists());
            assert!(journal.quarantine_path.exists());

            let restarted = ready_hot_registrar();
            assert!(recover_pending_purge_on_boot(&restarted).unwrap());
            assert!(lifecycle_store::load_purge_journal().unwrap().is_none());
            assert!(!journal.quarantine_path.exists());
        });
    }

    #[test]
    fn purge_agent_without_registered_root_fails_closed() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let registrar = ready_hot_registrar();
            start_agent_handler(
                json!({
                    "name": "claude",
                    "agent_type": "claude-code",
                }),
                &registrar,
            )
            .unwrap();

            let error = purge_agent_handler(json!({"name": "claude"}), &registrar)
                .expect_err("purge must not infer agents_root/name");

            assert!(error
                .to_string()
                .contains("has no `root_path`; refusing to infer"));
            assert!(agents::load_agents().unwrap().agents.contains_key("claude"));
            assert!(local_agents::lookup_hosted_ura(
                &local_agents::load().unwrap(),
                "llm",
                "claude"
            )
            .is_some());
        });
    }

    #[test]
    fn purge_agent_accepts_registered_root_with_custom_basename() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let registrar = ready_hot_registrar();
            let custom_root = config::home_dir()
                .join("project")
                .join("agent-runtime-directory");
            let response = start_agent_handler(
                json!({
                    "name": "claude",
                    "agent_type": "claude-code",
                    "root_path": custom_root,
                    "materialize_directory": true,
                }),
                &registrar,
            )
            .unwrap();
            let registered_root = std::path::PathBuf::from(response["root_path"].as_str().unwrap());
            assert_ne!(registered_root.file_name().unwrap(), "claude");

            let purged = purge_agent_handler(json!({"name": "claude"}), &registrar).unwrap();

            assert_eq!(purged["purge_state"], "purged");
            assert!(!registered_root.exists());
            assert!(!agents::load_agents().unwrap().agents.contains_key("claude"));
        });
    }

    #[test]
    fn stop_agent_rejects_destructive_parameter_and_preserves_root() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let registrar = ready_hot_registrar();
            let response = start_agent_handler(
                json!({
                    "name": "claude",
                    "agent_type": "claude-code",
                    "materialize_directory": true,
                }),
                &registrar,
            )
            .unwrap();
            let root = std::path::PathBuf::from(response["root_path"].as_str().unwrap());
            let error = stop_agent_handler(json!({"name": "claude", "purge": true}), &registrar)
                .expect_err("agent.stop must never accept destructive authority");

            assert!(error.to_string().contains("invoke `agent.purge`"));
            assert!(root.join("agent.toml").exists());
            assert!(agents::load_agents().unwrap().agents.contains_key("claude"));
        });
    }

    #[test]
    fn purge_post_rename_identity_mismatch_restores_swapped_root_atomically() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let registrar = ready_hot_registrar();
            let response = start_agent_handler(
                json!({
                    "name": "claude",
                    "agent_type": "claude-code",
                    "materialize_directory": true,
                }),
                &registrar,
            )
            .unwrap();
            let root = std::path::PathBuf::from(response["root_path"].as_str().unwrap());
            let parent = root.parent().unwrap().to_path_buf();
            let original_backup = parent.join("claude-original");
            let replacement = parent.join("claude-replacement");
            AgentDirectory::create(
                &Location::Local {
                    root: replacement.clone(),
                },
                AgentSpec::new("claude", RuntimeKind::ClaudeCode),
            )
            .unwrap();

            let registry = agents::load_agents().unwrap();
            let identities = local_agents::load().unwrap();
            let transaction_id = "toctou".to_string();
            let quarantine = parent.join(".claude.easynet-purge-toctou");
            let mut journal = AgentPurgeJournal::new(
                transaction_id,
                "claude".to_string(),
                hosted_agent_ura_from_file(&identities, "claude").unwrap(),
                root.clone(),
                quarantine.clone(),
                registry.agents["default/claude"].clone(),
                registry,
                identities,
            );
            let backup_for_hook = original_backup.clone();
            let replacement_for_hook = replacement.clone();
            PURGE_PRE_RENAME_HOOK.with(|slot| {
                *slot.borrow_mut() = Some(Box::new(move |registered_root| {
                    std::fs::rename(registered_root, &backup_for_hook).unwrap();
                    std::fs::rename(&replacement_for_hook, registered_root).unwrap();
                }));
            });

            let error = quarantine_registered_root(&mut journal)
                .expect_err("swapped inode must fail post-rename validation");

            assert!(error.to_string().contains("root restored"));
            assert!(root.join("agent.toml").exists());
            assert!(original_backup.join("agent.toml").exists());
            assert!(!quarantine.exists());
            lifecycle_store::clear_purge_journal().unwrap();
        });
    }

    #[cfg(unix)]
    #[test]
    fn committed_purge_finalize_failure_reports_discoverable_residual_path() {
        use std::os::unix::fs::PermissionsExt as _;

        with_isolated_home(|| {
            seed_joined_credentials();
            let registrar = ready_hot_registrar();
            let response = start_agent_handler(
                json!({
                    "name": "claude",
                    "agent_type": "claude-code",
                    "materialize_directory": true,
                }),
                &registrar,
            )
            .unwrap();
            let root = std::path::PathBuf::from(response["root_path"].as_str().unwrap());
            let parent = root.parent().unwrap().to_path_buf();
            let mut registry = agents::load_agents().unwrap();
            let removed_entry = registry.agents["default/claude"].clone();
            let mut identities = local_agents::load().unwrap();
            let quarantine = parent.join(".claude.easynet-purge-finalize-failure");
            let mut journal = AgentPurgeJournal::new(
                "finalize-failure".to_string(),
                "claude".to_string(),
                hosted_agent_ura_from_file(&identities, "claude").unwrap(),
                root,
                quarantine.clone(),
                removed_entry,
                registry.clone(),
                identities.clone(),
            );
            quarantine_registered_root(&mut journal).unwrap();
            registry.agents.remove("default/claude");
            agents::save_agents(&registry).unwrap();
            identities
                .hosted_agents
                .retain(|entry| !(entry.profile == "llm" && entry.name == "claude"));
            local_agents::save(&identities).unwrap();
            journal.stage = AgentPurgeStage::Committed;

            let original_mode = std::fs::metadata(&parent).unwrap().permissions().mode();
            std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o500)).unwrap();
            let result = finalize_committed_purge(&journal);
            std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(original_mode))
                .unwrap();
            let error = result.expect_err("read-only parent must prevent quarantine removal");

            assert!(error.to_string().contains("residual_path="));
            assert!(error
                .to_string()
                .contains(&quarantine.display().to_string()));
            assert!(quarantine.exists());
            finalize_committed_purge(&journal)
                .expect("committed finalize must resume after partial directory deletion");
            assert!(!quarantine.exists());
            lifecycle_store::clear_purge_journal().unwrap();
        });
    }

    #[cfg(unix)]
    #[test]
    fn committed_finalize_does_not_follow_quarantine_inode_swap() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let registrar = ready_hot_registrar();
            let response = start_agent_handler(
                json!({
                    "name": "claude",
                    "agent_type": "claude-code",
                    "materialize_directory": true,
                }),
                &registrar,
            )
            .unwrap();
            let root = std::path::PathBuf::from(response["root_path"].as_str().unwrap());
            let parent = root.parent().unwrap().to_path_buf();
            let mut registry = agents::load_agents().unwrap();
            let removed_entry = registry.agents["default/claude"].clone();
            let mut identities = local_agents::load().unwrap();
            let quarantine = parent.join(".claude.easynet-purge-finalize-swap");
            let moved_claim = parent.join(".claude.easynet-purge-open-inode");
            let mut journal = AgentPurgeJournal::new(
                "finalize-swap".to_string(),
                "claude".to_string(),
                hosted_agent_ura_from_file(&identities, "claude").unwrap(),
                root,
                quarantine.clone(),
                removed_entry,
                registry.clone(),
                identities.clone(),
            );
            quarantine_registered_root(&mut journal).unwrap();
            registry.agents.remove("default/claude");
            agents::save_agents(&registry).unwrap();
            identities
                .hosted_agents
                .retain(|entry| !(entry.profile == "llm" && entry.name == "claude"));
            local_agents::save(&identities).unwrap();

            let moved_for_hook = moved_claim.clone();
            PURGE_PRE_FINALIZE_HOOK.with(|slot| {
                *slot.borrow_mut() = Some(Box::new(move |claimed_path| {
                    std::fs::rename(claimed_path, &moved_for_hook).unwrap();
                    std::fs::create_dir(claimed_path).unwrap();
                    std::fs::write(claimed_path.join("must-survive"), b"replacement").unwrap();
                }));
            });

            let error = finalize_committed_purge(&journal)
                .expect_err("path replacement must fail identity-bound unlink");

            assert!(error.to_string().contains("identity changed"), "{error:#}");
            assert_eq!(
                std::fs::read(quarantine.join("must-survive")).unwrap(),
                b"replacement",
                "replacement directory must not be traversed or deleted"
            );
            assert!(moved_claim.exists(), "opened inode remains discoverable");
            std::fs::remove_dir_all(&quarantine).unwrap();
            std::fs::remove_dir_all(&moved_claim).unwrap();
            lifecycle_store::clear_purge_journal().unwrap();
        });
    }

    #[cfg(unix)]
    #[test]
    fn committed_finalize_detects_child_inode_swap_before_unlinkat() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let registrar = ready_hot_registrar();
            let response = start_agent_handler(
                json!({
                    "name": "child-swap",
                    "agent_type": "claude-code",
                    "materialize_directory": true,
                }),
                &registrar,
            )
            .unwrap();
            let root = std::path::PathBuf::from(response["root_path"].as_str().unwrap());
            let victim_dir = root.join("runs");
            std::fs::create_dir_all(&victim_dir).unwrap();
            std::fs::write(victim_dir.join("victim"), b"original").unwrap();

            PURGE_CHILD_ENTRY_HOOK.with(|slot| {
                *slot.borrow_mut() = Some(Box::new(move |name| {
                    if name != std::ffi::OsStr::new("victim") {
                        return false;
                    }
                    let journal = lifecycle_store::load_purge_journal()
                        .unwrap()
                        .expect("purge journal visible during child deletion");
                    let target = journal.quarantine_path.join("runs").join("victim");
                    let moved = journal.quarantine_path.join("runs").join("victim-original");
                    std::fs::rename(&target, &moved).unwrap();
                    std::fs::write(&target, b"replacement-must-survive").unwrap();
                    true
                }));
            });

            let error = purge_agent_handler(json!({"name": "child-swap"}), &registrar)
                .expect_err("child inode replacement must fail closed");
            assert!(
                error
                    .to_string()
                    .contains("changed identity before unlinkat"),
                "{error:#}"
            );
            let journal = lifecycle_store::load_purge_journal().unwrap().unwrap();
            let replacement = journal.quarantine_path.join("runs").join("victim");
            assert_eq!(
                std::fs::read(&replacement).unwrap(),
                b"replacement-must-survive"
            );
            assert_eq!(journal.stage, AgentPurgeStage::Committed);
        });
    }

    #[test]
    fn lifecycle_lock_prevents_cross_agent_stale_snapshot_lost_update() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let registrar = Arc::new(ready_hot_registrar());
            start_agent_handler(
                json!({"name": "old", "agent_type": "claude-code"}),
                registrar.as_ref(),
            )
            .unwrap();

            let barrier = Arc::new(std::sync::Barrier::new(3));
            let start_registrar = Arc::clone(&registrar);
            let start_barrier = Arc::clone(&barrier);
            let start = std::thread::spawn(move || {
                start_barrier.wait();
                start_agent_handler(
                    json!({"name": "new", "agent_type": "codex"}),
                    start_registrar.as_ref(),
                )
            });
            let stop_registrar = Arc::clone(&registrar);
            let stop_barrier = Arc::clone(&barrier);
            let stop = std::thread::spawn(move || {
                stop_barrier.wait();
                stop_agent_handler(json!({"name": "old"}), stop_registrar.as_ref())
            });
            barrier.wait();
            start.join().unwrap().unwrap();
            stop.join().unwrap().unwrap();

            let registry = agents::load_agents().unwrap();
            assert!(registry.agents.contains_key("default/new"));
            assert!(!registry.agents.contains_key("default/old"));
            let identities = local_agents::load().unwrap();
            assert!(local_agents::lookup_hosted_ura(&identities, "llm", "new").is_some());
            assert!(local_agents::lookup_hosted_ura(&identities, "llm", "old").is_none());
        });
    }

    #[test]
    fn concurrent_start_and_stop_of_one_agent_leave_registry_and_identity_consistent() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let registrar = Arc::new(ready_hot_registrar());
            start_agent_handler(
                json!({"name": "same", "agent_type": "claude-code"}),
                registrar.as_ref(),
            )
            .unwrap();

            let barrier = Arc::new(std::sync::Barrier::new(3));
            let start_registrar = Arc::clone(&registrar);
            let start_barrier = Arc::clone(&barrier);
            let start = std::thread::spawn(move || {
                start_barrier.wait();
                start_agent_handler(
                    json!({"name": "same", "agent_type": "codex"}),
                    start_registrar.as_ref(),
                )
            });
            let stop_registrar = Arc::clone(&registrar);
            let stop_barrier = Arc::clone(&barrier);
            let stop = std::thread::spawn(move || {
                stop_barrier.wait();
                stop_agent_handler(json!({"name": "same"}), stop_registrar.as_ref())
            });
            barrier.wait();
            start.join().unwrap().unwrap();
            stop.join().unwrap().unwrap();

            let registered = agents::load_agents().unwrap().agents.contains_key("same");
            let mapped =
                local_agents::lookup_hosted_ura(&local_agents::load().unwrap(), "llm", "same")
                    .is_some();
            assert_eq!(registered, mapped);
        });
    }

    #[test]
    fn lifecycle_lock_serializes_two_purges_of_the_same_agent() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let registrar = Arc::new(ready_hot_registrar());
            let response = start_agent_handler(
                json!({
                    "name": "claude",
                    "agent_type": "claude-code",
                    "materialize_directory": true,
                }),
                registrar.as_ref(),
            )
            .unwrap();
            let root = std::path::PathBuf::from(response["root_path"].as_str().unwrap());

            let barrier = Arc::new(std::sync::Barrier::new(3));
            let mut workers = Vec::new();
            for _ in 0..2 {
                let worker_registrar = Arc::clone(&registrar);
                let worker_barrier = Arc::clone(&barrier);
                workers.push(std::thread::spawn(move || {
                    worker_barrier.wait();
                    purge_agent_handler(json!({"name": "claude"}), worker_registrar.as_ref())
                }));
            }
            barrier.wait();
            let mut acknowledgements = workers
                .into_iter()
                .map(|worker| worker.join().unwrap().unwrap()["ack"].as_bool().unwrap())
                .collect::<Vec<_>>();
            acknowledgements.sort_unstable();

            assert_eq!(acknowledgements, vec![false, true]);
            assert!(!root.exists());
            assert!(!agents::load_agents().unwrap().agents.contains_key("claude"));
            assert!(lifecycle_store::load_purge_journal().unwrap().is_none());
        });
    }

    #[test]
    fn every_durable_purge_stage_recovers_to_a_deterministic_state() {
        with_isolated_home(|| {
            seed_joined_credentials();
            let registrar = ready_hot_registrar();
            let stages = [
                AgentPurgeStage::Prepared,
                AgentPurgeStage::Quarantined,
                AgentPurgeStage::RuntimeSynchronized,
                AgentPurgeStage::RegistryPersisted,
                AgentPurgeStage::IdentityPersisted,
                AgentPurgeStage::AuthorityCommitted,
                AgentPurgeStage::Committed,
                AgentPurgeStage::Finalized,
                AgentPurgeStage::TombstonePrepared,
                AgentPurgeStage::OutboxEnqueued,
            ];

            for (index, stage) in stages.into_iter().enumerate() {
                let name = format!("recovery-{index}");
                let response = start_agent_handler(
                    json!({
                        "name": name,
                        "agent_type": "claude-code",
                        "materialize_directory": true,
                    }),
                    &registrar,
                )
                .unwrap();
                let root = std::path::PathBuf::from(response["root_path"].as_str().unwrap());
                PURGE_CRASH_STAGE.with(|slot| slot.set(Some(stage)));
                let error = purge_agent_handler(json!({"name": name}), &registrar)
                    .expect_err("stage failpoint must interrupt purge");
                PURGE_CRASH_STAGE.with(|slot| slot.set(None));
                assert!(error.to_string().contains("injected Agent purge crash"));

                let journal = lifecycle_store::load_purge_journal()
                    .unwrap()
                    .expect("interrupted purge must remain discoverable");
                assert_eq!(journal.stage, stage);
                assert_eq!(journal.name, name);
                if stage == AgentPurgeStage::Prepared {
                    assert!(root.exists());
                    assert!(!journal.quarantine_path.exists());
                } else if matches!(
                    stage,
                    AgentPurgeStage::Finalized
                        | AgentPurgeStage::TombstonePrepared
                        | AgentPurgeStage::OutboxEnqueued
                ) {
                    assert!(!root.exists());
                    assert!(!journal.quarantine_path.exists());
                } else {
                    assert!(!root.exists());
                    assert!(journal.quarantine_path.exists());
                }

                refresh_agents_handler(json!({}), &registrar).unwrap();
                assert!(lifecycle_store::load_purge_journal().unwrap().is_none());
                assert!(!journal.quarantine_path.exists());
                let registered = agents::load_agents().unwrap().agents.contains_key(&name);
                if stage.is_committed() {
                    assert!(!registered);
                    assert!(!root.exists());
                } else {
                    assert!(registered);
                    assert!(root.join("agent.toml").exists());
                }
            }
        });
    }

    #[test]
    fn stop_agent_by_ura_removes_joined_hosted_mapping() {
        with_isolated_home(|| {
            seed_joined_credentials();
            start_agent_handler(
                json!({
                    "name": "anthropic",
                    "agent_type": "claude-code",
                    "materialize_directory": true,
                }),
                &ready_hot_registrar(),
            )
            .unwrap();

            let agent_ura = crate::core::ura::agent_ura("localhost", "user-dev", "anthropic");
            assert_eq!(
                local_agents::lookup_hosted_ura(&local_agents::load().unwrap(), "llm", "anthropic"),
                Some(agent_ura.clone())
            );

            let resp = stop_agent_handler(json!({"agent_ura": agent_ura}), &ready_hot_registrar())
                .unwrap();
            assert_eq!(resp["ack"], true);
            assert!(!agents::load_agents()
                .unwrap()
                .agents
                .contains_key("anthropic"));
            assert_eq!(
                local_agents::lookup_hosted_ura(&local_agents::load().unwrap(), "llm", "anthropic"),
                None
            );
        });
    }

    #[test]
    fn stop_agent_idempotent_returns_ack_false_when_row_missing() {
        with_isolated_home(|| {
            // Never registered; stop should report ack=false (not error).
            let resp =
                stop_agent_handler(json!({"name": "ghost"}), &ready_hot_registrar()).unwrap();
            assert_eq!(resp["ack"], false);
        });
    }

    #[test]
    fn stop_agent_by_agent_ura_resolves_agent_tail() {
        with_isolated_home(|| {
            seed_joined_credentials();
            start_agent_handler(
                json!({
                    "name": "claude",
                    "agent_type": "claude-code",
                }),
                &ready_hot_registrar(),
            )
            .unwrap();
            let agent_ura = crate::core::ura::agent_ura("localhost", "user-dev", "claude");
            let resp = stop_agent_handler(json!({"agent_ura": agent_ura}), &ready_hot_registrar())
                .unwrap();
            assert_eq!(resp["ack"], true);
            assert!(!agents::load_agents().unwrap().agents.contains_key("claude"));
        });
    }

    #[test]
    fn stop_agent_rejects_non_agent_ura() {
        let err = stop_agent_handler(
            json!({"agent_ura": crate::core::ura::device_ura("acme", "device-1")}),
            &ready_hot_registrar(),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("Agent URA"));
    }

    #[test]
    fn input_schemas_have_required_fields_pinned() {
        let s = start_agent_input_schema();
        let req = s["required"].as_array().unwrap();
        assert!(req.iter().any(|v| v == "name"));
        assert!(
            !req.iter().any(|v| v == "agent_type"),
            "top-level agent_type must stay optional because rich `entry` can carry it"
        );
        let alternatives = s["anyOf"].as_array().expect("start anyOf");
        assert!(alternatives
            .iter()
            .any(|shape| shape["required"][0] == "agent_type"));
        assert!(alternatives
            .iter()
            .any(|shape| shape["required"][0] == "entry"));
        assert!(s["properties"].get("entry").is_some());
        assert_eq!(s["properties"]["entry"]["required"][0], "agent_type");
        assert_eq!(s["additionalProperties"], false);

        let s = stop_agent_input_schema();
        let alternatives = s["anyOf"].as_array().expect("stop anyOf");
        assert!(alternatives
            .iter()
            .any(|shape| shape["required"][0] == "name"));
        assert!(alternatives
            .iter()
            .any(|shape| shape["required"][0] == "agent_ura"));
        assert!(s["properties"].get("name").is_some());
        assert!(s["properties"].get("agent_ura").is_some());
        assert!(s["properties"].get("purge").is_none());
        assert_eq!(s["additionalProperties"], false);

        let purge = purge_agent_input_schema();
        assert_eq!(purge, s);
        assert!(stop_agent_description().contains("always preserved"));
        assert!(purge_agent_description().contains("Destructively remove"));
        assert_ne!(ABILITY_STOP_AGENT, ABILITY_PURGE_AGENT);
        assert!(refresh_agents_description().contains("re-reads"));
    }
}
