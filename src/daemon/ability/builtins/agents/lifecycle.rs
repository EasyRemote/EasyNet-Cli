//! File: `src/daemon/ability/builtins/agents/lifecycle.rs`
//! Description: Transactional `agent.{start,stop,refresh}` handlers.
//!
//! Protocol responsibility: atomically converge the durable agent registry,
//! hosted identity index, authority inventory, ability catalog, and live Axon
//! runtime. A successful response means every local segment committed; Hub
//! advertisement remains best-effort but is always represented explicitly.
//!
//! Implementation approach: lifecycle mutations advance through an explicit
//! state machine and retain pre-mutation snapshots. Any local failure triggers
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
//   * agent.stop  — { name | agent_ura } → { ack: bool }
//                          ack=false when the row didn't exist
//                          (idempotent: callers can retry without
//                          triggering an error).
//
// What does NOT live here
// -----------------------
//   * Workspace cleanup. `easynet agent remove --purge` deletes
//     `~/.easynet/workspaces/<name>/`; the ability deliberately
//     doesn't, so a remote stop_agent can't accidentally wipe an
//     operator's local files. Workspace lifecycle stays under the
//     CLI subcommand.
//   * Process kill signals. There are no resident agent processes
//     today (see Lifecycle model above); a future per-agent
//     long-runner would land its own a future device.session operation.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::str::FromStr;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::core::agent::spec::{AgentSpec, RuntimeKind};
use crate::daemon::ability::catalog::profiles::bootstrap::{
    self, BootstrapPlan, LlmSubAgent, UuidMinter,
};
use crate::daemon::ability::dispatch::AxonAbilityCatalog;
use crate::daemon::axon_bridge::hot_agent_registrar::{
    block_on_hot_registrar, HotAgentAdvertiseRequest, HotAgentAdvertiseState, HotAgentRegistrar,
    HotAgentRegistrarError,
};
use crate::daemon::execution::mission::directory::{AgentDirectory, Location};
use crate::daemon::persistence::agent_registry as agents;
use crate::daemon::persistence::agent_registry::{
    AgentEntry, AgentRegistry, AgentType, CURRENT_REGISTRY_SCHEMA,
};
use crate::daemon::persistence::{config, local_agents};

use crate::daemon::ability::dispatch::OwnerKind;
pub const ABILITY_START_AGENT: &str = crate::daemon::ability::names::agents::AGENT_START;
pub const ABILITY_STOP_AGENT: &str = crate::daemon::ability::names::agents::AGENT_STOP;
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
            Self::Committed => "committed",
            Self::RollingBack => "rolling_back",
            Self::RolledBack => "rolled_back",
            Self::PartialFailure => "partial_failure",
        })
    }
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

struct AgentLifecycleTransaction {
    operation: &'static str,
    state: AgentLifecycleState,
    original_registry: AgentRegistry,
    original_local_agents: local_agents::LocalAgentsFile,
    registry_written: bool,
    identity_written: bool,
    materialization: MaterializationRollback,
}

impl AgentLifecycleTransaction {
    fn new(
        operation: &'static str,
        original_registry: AgentRegistry,
        original_local_agents: local_agents::LocalAgentsFile,
    ) -> Self {
        Self {
            operation,
            state: AgentLifecycleState::Prepared,
            original_registry,
            original_local_agents,
            registry_written: false,
            identity_written: false,
            materialization: MaterializationRollback::None,
        }
    }

    fn record_materialization(&mut self, rollback: MaterializationRollback) {
        self.materialization = rollback;
        self.state = AgentLifecycleState::Materialized;
    }

    fn persist(
        &mut self,
        registry: &AgentRegistry,
        identities: &local_agents::LocalAgentsFile,
    ) -> Result<(), AgentLifecycleError> {
        // Mark each segment before the atomic-write call: a post-rename
        // directory-sync error means the new bytes may already be visible and
        // therefore still require compensation.
        self.registry_written = true;
        if let Err(error) = agents::save_agents(registry) {
            return Err(
                self.failure_with_rollback(format!("persist durable agent registry: {error:#}"))
            );
        }
        self.state = AgentLifecycleState::DurablePersisted;
        self.identity_written = true;
        if let Err(error) = local_agents::save(identities) {
            return Err(self.failure_with_rollback(format!(
                "persist hosted-Agent identity registry: {error:#}"
            )));
        }
        self.state = AgentLifecycleState::IdentityPersisted;
        Ok(())
    }

    fn mark_runtime_synchronized(&mut self) {
        self.state = AgentLifecycleState::RuntimeSynchronized;
    }

    fn commit(&mut self) {
        self.state = AgentLifecycleState::Committed;
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
        self.state = AgentLifecycleState::RollingBack;
        let mut failures = Vec::new();
        if self.identity_written {
            if let Err(error) = local_agents::save(&self.original_local_agents) {
                failures.push(format!("restore local-agents.json: {error:#}"));
            }
        }
        if self.registry_written {
            if let Err(error) = agents::save_agents(&self.original_registry) {
                failures.push(format!("restore agents.json: {error:#}"));
            }
        }
        if let Err(error) = self.materialization.rollback() {
            failures.push(format!("restore agent directory: {error:#}"));
        }
        self.state = if failures.is_empty() {
            AgentLifecycleState::RolledBack
        } else {
            AgentLifecycleState::PartialFailure
        };
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
    let registrar_for_refresh = Arc::clone(&hot_registrar);
    reg.register_rpc_with_owner(
        "agent.refresh",
        OwnerKind::Device,
        Arc::new(move |args: Value| refresh_agents_handler(args, &registrar_for_refresh)),
    );
}

/// `agent.start` handler.
///
/// Args: `{ "name": "claude", "agent_type": "claude-code", "model": "sonnet"? }`
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
    let name = args
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("agent.start: `name` (non-empty string) required"))?
        .to_string();
    // DEC-F048: hosted user agent ≠ device-sponsored System Agent.
    if crate::daemon::axon_bridge::hot_agent_registrar::name_claims_reserved_device_owner(&name) {
        anyhow::bail!(
            "agent.start: `device.` is the reserved owner token for \
             device-sponsored System Agents (RFC-005 §3.1.2, DEC-F048); \
             hosted user agents cannot take a device-owned identity — \
             choose a name that is not `device` and does not begin with `device.`"
        );
    }
    let model = args
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string);
    let model_present = args
        .get("model_present")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| args.get("model").is_some());
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
    let original_local_agents = local_agents::load()
        .map_err(|error| anyhow::anyhow!("agent.start: load hosted-Agent identities: {error:#}"))?;
    let mut transaction = AgentLifecycleTransaction::new(
        "agent.start",
        original_registry.clone(),
        original_local_agents.clone(),
    );
    let mut registry = original_registry;
    let existing_entry = registry.agents.get(&name).cloned();
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
                });
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
            });
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
        entry.root_path = Some(directory.root().to_path_buf());
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
    registry.agents.insert(name.clone(), entry.clone());
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
    transaction.mark_runtime_synchronized();
    transaction.commit();

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
    let owner_projection_descriptors = build_hot_agent_descriptors(&name, &entry, &agent_ura);
    let mut abilities_payload: Option<Vec<u8>> = None;
    let mut owner_projection_state = "not_configured";
    let mut owner_projection_error: Option<String> = None;
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

    let hub_advertise_outcome = registrar.hot_agent_advertiser().map(|advertiser| {
        advertiser.advertise_hosted_agent(HotAgentAdvertiseRequest {
            agent_ura: agent_ura.clone(),
            abilities_payload: abilities_payload.clone(),
        })
    });
    if let Some(outcome) = hub_advertise_outcome.as_ref() {
        if let Some(err) = outcome.error() {
            crate::op_event!(
                component = agent_lifecycle,
                kind = hot_agent_hub_advertise_soft_failed,
                agent_name = name.as_str(),
                agent_ura = agent_ura.as_str(),
                error = err,
                message = "agent registered locally but hub advertise failed; \
                           frontend remote invokes may need a session reconnect",
            );
        }
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
        "hub_advertised": hub_advertise_outcome
            .as_ref()
            .map(|outcome| outcome.advertised())
            .unwrap_or(false),
        "hub_advertise_state": match hub_advertise_outcome.as_ref() {
            None => "not_configured",
            Some(outcome) if outcome.state() == HotAgentAdvertiseState::Succeeded => "succeeded",
            Some(_) => "failed",
        },
        "hub_advertise_error": hub_advertise_outcome
            .as_ref()
            .and_then(|outcome| outcome.error().map(str::to_string)),
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

/// Build the owner-projection ability descriptors for one hot-added
/// agent, using the SAME construction as the boot-time republish path
/// (`daemon::federation::publish` step 5b): `abilities_for_publication` →
/// owner-local public name → `AbilityDescriptor`. Kept byte-equivalent
/// to boot so the hot-add path is not a second, lossy catalogue (the
/// divergence that previously omitted newly-added abilities from
/// `namespace.resolve`). ISS-002.
fn build_hot_agent_descriptors(
    name: &str,
    entry: &AgentEntry,
    agent_ura: &str,
) -> Vec<crate::daemon::ability::descriptors::AbilityDescriptor> {
    let live_registry = crate::daemon::ability::catalog::build_registry();
    let mut descriptors = Vec::new();
    for spec in crate::daemon::execution::mission::agent_ability_specs::abilities_for_publication(
        name, entry,
    ) {
        let registry_name = spec.name();
        match crate::daemon::ability::catalog::profiles::llm::descriptor_for_agent_spec(
            &live_registry,
            agent_ura,
            name,
            &spec,
        ) {
            Ok(desc) => {
                let mut desc = desc
                    .with_metadata_entry("runtime", entry.agent_type.to_string())
                    .with_metadata_entry("agent_type", entry.agent_type.to_string())
                    .with_metadata_entry("base_runtime", entry.agent_type.to_string());
                if let Some(model) = entry.model.as_ref() {
                    desc = desc
                        .with_metadata_entry("model", model.clone())
                        .with_metadata_entry("base_model", model.clone());
                }
                descriptors.push(desc);
            }
            Err(err) => crate::op_event!(
                component = agent_lifecycle,
                kind = hot_agent_descriptor_build_failed,
                agent_name = name,
                agent_ura = agent_ura,
                ability = registry_name,
                error = err.to_string().as_str(),
                message = "skipped one ability descriptor for the hot-added agent's \
                           owner projection; remaining abilities still publish",
            ),
        }
    }
    descriptors
}

fn runtime_kind_from(t: AgentType) -> RuntimeKind {
    t.runtime_kind()
}

fn normalize_v2_entry(entry: &mut AgentEntry) {
    entry.command.clear();
    entry.args.clear();
    entry.env.clear();
    entry.timeout_secs = agents::default_timeout_for_new_rows();
    entry.max_output_bytes = agents::default_max_output_for_new_rows();
    entry.schema_version = CURRENT_REGISTRY_SCHEMA;
}

/// `agent.stop` handler.
///
/// Args: `{ "name": "claude" }` or `{ "agent_ura": "easynet:///r/<realm>/agent/<user>.claude" }`.
/// Behaviour: remove the registry row. Idempotent — `ack=false` if
/// the row didn't exist; never errors on missing target.
fn stop_agent_handler(
    args: Value,
    hot_registrar: &SharedHotRegistrarCell,
) -> anyhow::Result<Value> {
    let original_registry = agents::load_agents()
        .map_err(|error| anyhow::anyhow!("agent.stop: load durable agent registry: {error:#}"))?;
    let name = stop_agent_name_from_args(&args)?;
    let Some(removed_entry) = original_registry.agents.get(&name).cloned() else {
        return Ok(json!({
            "ack": false,
            "runtime_removed": 0,
            "removed_entry": Value::Null,
            "hub_tombstone_state": "not_applicable",
            "hub_tombstone_error": Value::Null,
            "hub_revoke_state": "not_applicable",
            "hub_revoke_error": Value::Null,
        }));
    };
    let registrar = require_hot_registrar(hot_registrar, "agent.stop")?;
    let original_local_agents = local_agents::load()
        .map_err(|error| anyhow::anyhow!("agent.stop: load hosted-Agent identities: {error:#}"))?;
    let agent_ura = hosted_agent_ura_from_file(&original_local_agents, &name)
        .map_err(|error| anyhow::anyhow!("agent.stop: {error:#}"))?;

    // Phase one removes runtime/catalog rows while durable lifecycle state
    // still proves that this daemon owns the Agent authority.
    let registrar_for_remove = Arc::clone(&registrar);
    let name_for_remove = name.clone();
    let entry_for_remove = removed_entry.clone();
    let removal = block_on_hot_registrar(async move {
        registrar_for_remove
            .unregister_agent(&name_for_remove, &entry_for_remove)
            .await
    })
    .map_err(|source| AgentLifecycleError::Registrar {
        operation: "agent.stop",
        source: Box::new(source),
    })?;
    let runtime_removed = removal.outcome().removed;

    let mut registry = original_registry.clone();
    registry.agents.remove(&name);
    let mut identities = original_local_agents.clone();
    identities
        .hosted_agents
        .retain(|entry| !(entry.profile == "llm" && entry.name == name));
    let mut transaction =
        AgentLifecycleTransaction::new("agent.stop", original_registry, original_local_agents);
    transaction.mark_runtime_synchronized();
    if let Err(error) = transaction.persist(&registry, &identities) {
        let registrar_for_restore = Arc::clone(&registrar);
        let name_for_restore = name.clone();
        let entry_for_restore = removed_entry.clone();
        let restore = block_on_hot_registrar(async move {
            registrar_for_restore
                .register_agent(&name_for_restore, &entry_for_restore)
                .await
        });
        return match restore {
            Ok(_) => Err(error.into()),
            Err(restore_error) => Err(AgentLifecycleError::Mutation {
                operation: "agent.stop",
                state: AgentLifecycleState::PartialFailure,
                cause: error.to_string(),
                rollback: format!("partial(restore runtime/catalog: {restore_error})"),
            }
            .into()),
        };
    }

    // Phase two proves both lifecycle rows are gone before revoking authority.
    if let Err(error) = registrar.commit_agent_removal(&removal) {
        let rollback_failures = transaction.rollback();
        let registrar_for_restore = Arc::clone(&registrar);
        let name_for_restore = name.clone();
        let entry_for_restore = removed_entry.clone();
        let runtime_restore = block_on_hot_registrar(async move {
            registrar_for_restore
                .register_agent(&name_for_restore, &entry_for_restore)
                .await
        });
        let mut failures = rollback_failures;
        if let Err(restore_error) = runtime_restore {
            failures.push(format!("restore runtime/catalog: {restore_error}"));
        }
        return Err(AgentLifecycleError::Mutation {
            operation: "agent.stop",
            state: AgentLifecycleState::IdentityPersisted,
            cause: format!("revoke hosted-Agent authority: {error}"),
            rollback: if failures.is_empty() {
                "completed".to_string()
            } else {
                format!("partial({})", failures.join("; "))
            },
        }
        .into());
    }
    transaction.commit();

    // ISS-002 closed loop (stop side, symmetric to start): tell the hub
    // the agent's abilities are gone NOW instead of waiting for the next
    // heartbeat. We advertise an empty complete-set so the hub's
    // complete-set REPLACE tombstones every prior projected ability
    // (removed = old − ∅), and we drop the local cursor so the owner
    // leaves the heartbeat refresh batch. Best-effort: failures degrade
    // to "reconciles on next boot/heartbeat" + an op_event.
    let mut hub_tombstone_state = "not_configured";
    let mut hub_tombstone_error: Option<String> = None;
    let mut hub_revoke_state = "not_configured";
    let mut hub_revoke_error: Option<String> = None;
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
                        let outcome = advertiser.advertise_hosted_agent(HotAgentAdvertiseRequest {
                            agent_ura: agent_ura.clone(),
                            abilities_payload: Some(payload),
                        });
                        hub_tombstone_state =
                            if outcome.state() == HotAgentAdvertiseState::Succeeded {
                                "succeeded"
                            } else {
                                "failed"
                            };
                        hub_tombstone_error = outcome.error().map(str::to_string);
                        if let Some(err) = outcome.error() {
                            crate::op_event!(
                                component = agent_lifecycle,
                                kind = hot_agent_stop_tombstone_soft_failed,
                                agent_name = name.as_str(),
                                agent_ura = agent_ura.as_str(),
                                error = err,
                                message = "agent stopped locally but hub ability \
                                               tombstone advertise failed; hub reconciles \
                                               on next heartbeat refresh",
                            );
                        }
                    }
                    (Ok(_), None) => {}
                    (Err(error), _) => {
                        hub_tombstone_state = "failed";
                        hub_tombstone_error = Some(error);
                    }
                }
            }
            Ok(None) => hub_tombstone_state = "not_applicable",
            Err(err) => {
                hub_tombstone_state = "failed";
                hub_tombstone_error = Some(err.clone());
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
        if let Some(advertiser) = advertiser.as_ref() {
            let outcome = advertiser.revoke_hosted_agent(
                crate::daemon::axon_bridge::hot_agent_registrar::HotAgentRevokeRequest {
                    agent_ura: agent_ura.clone(),
                    reason: "agent.stop".to_string(),
                },
            );
            hub_revoke_state = if outcome.state() == HotAgentAdvertiseState::Succeeded {
                "succeeded"
            } else {
                "failed"
            };
            hub_revoke_error = outcome.error().map(str::to_string);
            if let Some(err) = outcome.error() {
                crate::op_event!(
                    component = agent_lifecycle,
                    kind = hot_agent_stop_revoke_soft_failed,
                    agent_name = name.as_str(),
                    agent_ura = agent_ura.as_str(),
                    error = err,
                    message = "agent stopped locally but hub identity revoke \
                                   failed; the agent record may linger in the hub \
                                   directory until operator revoke or hub restart",
                );
            }
        }
    }

    Ok(json!({
        "ack": true,
        "runtime_removed": runtime_removed,
        "removed_entry": removed_entry,
        "hub_tombstone_state": hub_tombstone_state,
        "hub_tombstone_error": hub_tombstone_error,
        "hub_revoke_state": hub_revoke_state,
        "hub_revoke_error": hub_revoke_error,
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
            let entry = registry.agents.get(name).cloned().ok_or_else(|| {
                anyhow::anyhow!("agent.refresh: agent {name:?} is not registered")
            })?;
            vec![(name.clone(), entry)]
        }
        None => registry
            .agents
            .iter()
            .map(|(name, entry)| (name.clone(), entry.clone()))
            .collect(),
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
    let username = credentials.username_slug()?.to_string();
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
        username,
        host_device_ura,
        consent: true,
        mcp: false,
        llm_sub_agents: registry
            .agents
            .iter()
            .map(|(name, entry)| LlmSubAgent {
                name: name.clone(),
                agent_type_display: entry.agent_type.to_string(),
                model: entry.model.clone(),
            })
            .collect(),
    })
}

fn stop_agent_name_from_args(args: &Value) -> anyhow::Result<String> {
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
        (None, Some(ura)) => agent_name_from_ura(ura),
        (Some(name), Some(ura)) => {
            let from_ura = agent_name_from_ura(ura)?;
            if from_ura != name {
                anyhow::bail!(
                    "agent.stop: `name` ({name}) does not match `agent_ura` ({from_ura})"
                );
            }
            Ok(name.to_string())
        }
        (None, None) => {
            anyhow::bail!("agent.stop: either `name` or `agent_ura` is required")
        }
    }
}

fn agent_name_from_ura(ura: &str) -> anyhow::Result<String> {
    let parsed = crate::core::ura::parse_ura(ura)
        .map_err(|err| anyhow::anyhow!("agent.stop: invalid `agent_ura`: {err}"))?;
    if parsed.kind != crate::core::ura::URAKind::Agent {
        anyhow::bail!("agent.stop: `agent_ura` must be an Agent URA");
    }
    // DEC-F048: device-sponsored System Agents are not hosted user
    // agents — they cannot be registered here (see the agent.start
    // gate), so a lifecycle reference to one is a category error,
    // not a missing-agent case.
    if parsed.device_agent_ids().is_some() {
        anyhow::bail!(
            "agent.stop: {ura} is a device-sponsored System Agent \
             (RFC-005 §3.1.2, DEC-F048); System Agents are not \
             lifecycle-managed as hosted agents on this surface"
        );
    }
    let identities = crate::daemon::persistence::local_agents::load()
        .map_err(|error| anyhow::anyhow!("agent.stop: load hosted-Agent identities: {error:#}"))?;
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
     Idempotent: ack=false when the row didn't exist. Workspace files \
     are deliberately NOT deleted — use `easynet agent remove --purge` for that."
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
        ready_hot_registrar_fixture(None, "localhost").cell
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

    #[derive(Default)]
    struct RecordingHotAdvertiser {
        requests: std::sync::Mutex<Vec<String>>,
    }

    impl crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiser
        for RecordingHotAdvertiser
    {
        fn advertise_hosted_agent(
            &self,
            request: crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseRequest,
        ) -> crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome {
            self.requests.lock().unwrap().push(request.agent_ura);
            crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome::succeeded()
        }

        fn revoke_hosted_agent(
            &self,
            request: crate::daemon::axon_bridge::hot_agent_registrar::HotAgentRevokeRequest,
        ) -> crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome {
            self.requests.lock().unwrap().push(request.agent_ura);
            crate::daemon::axon_bridge::hot_agent_registrar::HotAgentAdvertiseOutcome::succeeded()
        }
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
        runtime: Arc<easynet_axon::invocation::LocalRuntime>,
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
                    crate::daemon::ability::builtins::agents::discover::BridgeDiscoverFederationResolver,
                ),
            );
        let runtime = easynet_axon::invocation::LocalRuntime::new();
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
        let mut reg = AxonAbilityCatalog::new();
        register(&mut reg, Arc::new(ready_hot_registrar()));
        assert!(reg.get_rpc(ABILITY_START_AGENT).is_some());
        assert!(reg.get_rpc(ABILITY_STOP_AGENT).is_some());
        assert!(reg.get_rpc(ABILITY_REFRESH_AGENTS).is_some());
    }

    #[test]
    fn stop_agent_rejects_device_sponsored_system_agent_ura() {
        with_isolated_home(|| {
            let err = agent_name_from_ura("easynet:///r/localhost/agent/device.dev-1.terminal")
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
    fn start_agent_materialize_syncs_hosted_ura_and_default_chat_manifest() {
        with_isolated_home(|| {
            seed_joined_credentials();

            let resp = start_agent_handler(
                json!({
                    "name": "anthropic",
                    "agent_type": "claude-code",
                    "model": "sonnet",
                    "materialize_directory": true,
                }),
                &ready_hot_registrar(),
            )
            .unwrap();

            let expected_ura = crate::core::ura::agent_ura("localhost", "dev", "anthropic");
            assert_eq!(resp["agent_ura"], json!(expected_ura));
            assert_eq!(
                local_agents::lookup_hosted_ura(&local_agents::load().unwrap(), "llm", "anthropic"),
                Some(expected_ura),
                "newly added agents must be visible to hosted-agent descriptor synthesis"
            );

            let registry = agents::load_agents().unwrap();
            let root = registry.agents["anthropic"].root_path.clone().unwrap();
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
                "forged".to_string(),
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
                .contains_key("rollback-worker"));
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
            let agent_ura = crate::core::ura::agent_ura("localhost", "dev", "rollback-worker");
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

        let expected_ura = crate::core::ura::agent_ura("localhost", "dev", "anthropic");
        assert_eq!(resp["hub_advertised"], true);
        assert_eq!(resp["hub_advertise_error"], Value::Null);
        assert_eq!(
            advertiser.requests.lock().unwrap().as_slice(),
            [expected_ura.as_str()],
            "hot-added agent must be advertised to the hub immediately"
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
                registry.agents.get("claude").unwrap().agent_type,
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
            let stored = registry.agents.get("semop").unwrap();
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
            let stored = registry.agents.get("codex-rich").unwrap();
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

            let stored_root = agents::load_agents().unwrap().agents["claude"]
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

            let agent_ura = crate::core::ura::agent_ura("localhost", "dev", "anthropic");
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
            let agent_ura = crate::core::ura::agent_ura("localhost", "dev", "claude");
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
        assert_eq!(s["additionalProperties"], false);
        assert!(refresh_agents_description().contains("re-reads"));
    }
}
