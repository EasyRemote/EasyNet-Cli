//! File: `src/daemon/axon_bridge/hot_agent_registrar.rs`
//! Description: Transactional hot registration for daemon-hosted Agents.
//!
//! Protocol responsibility: turn one validated hosted-Agent lifecycle record
//! into the canonical authority, catalogue/control-plane, and `LocalRuntime`
//! rows for `<agent>.*`, and remove those rows symmetrically on stop.
//!
//! Implementation approach: the registrar exposes an explicit readiness state
//! (`PendingRuntime -> PendingCatalog -> Ready`) and fails closed until Ready.
//! Registration first enrolls the Agent authority through the catalogue-owned
//! durable authority inventory, then writes dynamic ability rows. Removal uses
//! an opaque token: runtime/catalogue rows are removed while the durable record
//! still proves authority; inventory revocation is committed only after both
//! lifecycle stores prove the Agent is gone. Multi-row failures carry typed
//! partial outcomes and reverse-order rollback status.
//!
//! Architecture: this module owns orchestration and handler synthesis only.
//! [`AxonAbilityCatalog`] remains the single writer for descriptor, authority,
//! implementation, execution-index, and runtime facts. Lifecycle persistence
//! remains owned by `agents::lifecycle`; Hub advertisement is a separate,
//! best-effort observer and never changes local commit semantics.

use std::sync::{Arc, OnceLock};

use axon_sdk::invocation::LocalRuntime;

use crate::daemon::ability::builtins::agents::chat::{
    build_agent_ability_handler, build_chat_handler_for, build_chat_stream_handler_for,
    build_discover_handler_for, build_host_stream_handler, ContextLoader,
};
use crate::daemon::ability::dispatch::{AxonAbilityCatalog, OwnerKind};
use crate::daemon::persistence::agent_registry::AgentEntry;

pub(crate) fn block_on_hot_registrar<F, T>(future: F) -> T
where
    F: std::future::Future<Output = T> + Send,
    T: Send,
{
    crate::support::async_bridge::run_blocking(
        future,
        crate::support::async_bridge::SyncBridgeRuntimePolicy::BuildCurrentThreadTokio,
    )
}

#[derive(Debug, Clone)]
struct HostedAgentRuntimeBinding {
    agent_ura: String,
}

struct HotAgentRuntimeSyncContext<'a> {
    runtime: &'a Arc<LocalRuntime>,
    catalog: &'a Arc<AxonAbilityCatalog>,
    binding: &'a HostedAgentRuntimeBinding,
    authority_scope: crate::daemon::ability::AuthorityScope,
    outcome: &'a mut HotAgentRuntimeSyncOutcome,
    failures: &'a mut Vec<HotAgentRuntimeSyncFailure>,
}

impl HostedAgentRuntimeBinding {
    #[cfg(test)]
    fn load(name: &str) -> anyhow::Result<Self> {
        use crate::daemon::persistence::agent_aggregate::AgentAggregateRepository;

        let snapshot = AgentAggregateRepository::try_load_snapshot()
            .map_err(|err| anyhow::anyhow!("load Agent aggregate for hosted agent: {err:#}"))?;
        let agent_ura = snapshot
            .hosted_agent_ura_by_name(name)
            .map_err(|err| anyhow::anyhow!("{err}"))?
            .ok_or_else(|| {
                anyhow::anyhow!("hosted agent {name:?} is missing from Agent aggregate")
            })?;
        let parsed = crate::core::ura::parse_ura(agent_ura)
            .map_err(|err| anyhow::anyhow!("invalid hosted agent URA {:?}: {err}", agent_ura))?;
        if parsed.kind != crate::core::ura::URAKind::Agent {
            anyhow::bail!(
                "hosted agent {name:?} resolved to non-Agent URA {:?}",
                agent_ura
            );
        }
        let Some((_, agent_id)) = parsed.agent_ids().or_else(|| parsed.device_agent_ids()) else {
            anyhow::bail!(
                "hosted agent {name:?} URA {:?} does not expose an agent id",
                agent_ura
            );
        };
        if agent_id != name {
            anyhow::bail!(
                "hosted agent {name:?} resolved to URA {:?} with mismatched agent id {agent_id:?}",
                agent_ura
            );
        }
        Ok(Self {
            agent_ura: agent_ura.to_string(),
        })
    }

    fn runtime_ability_ura(&self, registry_ability: &str) -> Option<String> {
        let public_name =
            crate::core::ura::owner_local_ability_name(&self.agent_ura, registry_ability);
        crate::core::ura::owner_ability_ura(&self.agent_ura, &public_name)
    }

    fn authority_scope(
        &self,
        agent_name: &str,
    ) -> Result<
        crate::daemon::ability::AuthorityScope,
        crate::daemon::ability::AbilityControlPlaneError,
    > {
        crate::daemon::ability::AuthorityScope::new(
            format!("agent:{agent_name}"),
            self.agent_ura.clone(),
        )
    }
}

fn dispatch_key_for_hosted_agent_runtime_key(
    runtime_key: &str,
    expected_agent: &str,
) -> Option<String> {
    let selector = crate::core::ura::AbilitySelector::parse(runtime_key).ok()?;
    let parsed_owner = crate::core::ura::parse_ura(selector.owner_ura()).ok()?;
    if parsed_owner.kind != crate::core::ura::URAKind::Agent {
        return None;
    }
    let (_, agent_id) = parsed_owner
        .agent_ids()
        .or_else(|| parsed_owner.device_agent_ids())?;
    if agent_id != expected_agent {
        return None;
    }
    Some(crate::core::ura::local_dispatch_ability_key(
        selector.owner_ura(),
        selector.public_name(),
    ))
}

fn hot_agent_runtime_surface_name(agent_identifier: &str) -> Result<String, String> {
    if agent_identifier.contains('/') {
        let agent_id = crate::core::agent::id::AgentId::parse(agent_identifier)
            .map_err(|error| error.to_string())?;
        if agent_id.to_string() != agent_identifier {
            return Err(format!(
                "registry key is not canonical; expected {:?}",
                agent_id.to_string()
            ));
        }
        return Ok(agent_id.name);
    }
    crate::core::agent::id::AgentId::new(crate::core::agent::id::DEFAULT_TENANT, agent_identifier)
        .map(|agent| agent.name)
        .map_err(|error| error.to_string())
}

async fn hosted_agent_runtime_ability_uras_for_agent(
    runtime: &Arc<LocalRuntime>,
    agent: &str,
) -> Vec<String> {
    let prefix = format!("{agent}.");
    runtime
        .list_abilities()
        .await
        .into_iter()
        .filter_map(|descriptor| {
            let dispatch_key = dispatch_key_for_hosted_agent_runtime_key(&descriptor.name, agent)?;
            dispatch_key.starts_with(&prefix).then_some(descriptor.name)
        })
        .collect()
}

/// Captures every dependency a hot-add path needs to synthesise an
/// agent's handler set + register it into the ability catalogue.
///
/// Constructed during catalogue assembly, then wired exactly once with the
/// shared runtime and completed catalogue. The pending construction shape
/// solves the closure cycle only; public lifecycle calls fail closed until
/// [`Self::readiness`] is `Ready`.
pub struct HotAgentRegistrar {
    /// Populated exactly once by catalogue assembly. Missing state is reported
    /// as [`HotAgentRegistrarReadiness::PendingRuntime`].
    runtime: OnceLock<Arc<LocalRuntime>>,
    loaders: Arc<Vec<Arc<dyn ContextLoader>>>,
    /// The discover + invoke handlers re-enter local dispatch through
    /// this handle to resolve peer-agent ability descriptors.
    dispatch_handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>>,
    /// Federation resolver used by hot-added `<agent>.discover`
    /// handlers. Must match the boot-time handler dependency so hot
    /// agents do not observe a different user/public tier.
    discover_federation_resolver:
        crate::daemon::ability::builtins::agents::discover::SharedDiscoverFederationResolver,
    /// Optional hub-advertise bridge for hot-added hosted agents.
    ///
    /// Runtime registration is local; hub visibility is separate.
    /// Device-mode boot wires this after the long-lived
    /// `session.open` escalation channel exists. Tests and
    /// non-device modes leave it empty. Local lifecycle success is independent
    /// of Hub reachability; the public lifecycle response reports
    /// `not_configured`, `succeeded`, or `failed` explicitly.
    hot_advertiser: OnceLock<Arc<dyn HotAgentAdvertiser>>,
}

/// True when `name` claims the reserved `device.` owner token — the
/// grammar slot for device-sponsored System Agents
/// (`agent/device.<device-id>.<agent-id>`, RFC-005 §3.1.2 / DEC-F048).
/// Hosted user agents MUST NOT register under it: a hosted agent named
/// `device.<x>` would mint `device.<x>.*` runtime rows that read as
/// device-owned ability shapes downstream.
#[must_use]
pub fn name_claims_reserved_device_owner(name: &str) -> bool {
    name == "device" || name.starts_with("device.")
}

/// Registrar readiness is a monotonic boot state. Lifecycle mutations are
/// legal only in `Ready`; pending states are typed failures, never successful
/// no-ops repaired by a later restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotAgentRegistrarReadiness {
    PendingRuntime,
    PendingCatalog,
    Ready,
}

impl std::fmt::Display for HotAgentRegistrarReadiness {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::PendingRuntime => "pending_runtime",
            Self::PendingCatalog => "pending_catalog",
            Self::Ready => "ready",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotAgentSyncOperation {
    Register,
    Unregister,
}

impl std::fmt::Display for HotAgentSyncOperation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Register => "register",
            Self::Unregister => "unregister",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotAgentRuntimeSyncFailure {
    pub ability: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotAgentRollbackStatus {
    Completed,
    Partial { failures: Vec<String> },
}

impl std::fmt::Display for HotAgentRollbackStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Completed => formatter.write_str("completed"),
            Self::Partial { failures } => write!(formatter, "partial({})", failures.join("; ")),
        }
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum HotAgentRegistrarError {
    #[error("hot hosted-Agent registrar is not ready: {readiness}")]
    NotReady {
        readiness: HotAgentRegistrarReadiness,
    },
    #[error("hosted Agent identifier {agent:?} is invalid for runtime registration: {reason}")]
    InvalidAgentIdentifier { agent: String, reason: String },
    #[error("hosted Agent name {agent:?} claims the reserved device owner namespace")]
    ReservedOwner { agent: String },
    #[error("hosted Agent {agent:?} authority enrollment failed: {reason}")]
    AuthorityEnrollment { agent: String, reason: String },
    #[error("hosted Agent {agent:?} authority scope is invalid: {reason}")]
    AuthorityScope { agent: String, reason: String },
    #[error(
        "hot hosted-Agent {operation} failed for {agent:?} after {failure_count} row error(s); rollback={rollback}"
    )]
    RuntimeSync {
        operation: HotAgentSyncOperation,
        agent: String,
        failure_count: usize,
        outcome: HotAgentRuntimeSyncOutcome,
        failures: Vec<HotAgentRuntimeSyncFailure>,
        rollback: HotAgentRollbackStatus,
    },
    #[error("commit hosted Agent {agent:?} authority revocation failed: {reason}")]
    AuthorityRevocation { agent: String, reason: String },
    #[error("hot hosted-Agent registrar wiring rejected a second {dependency} writer")]
    SecondWriter { dependency: &'static str },
}

/// Successful row counts for a registrar transaction. `failed` is retained in
/// the public response shape, but a successful `Result` always carries zero;
/// non-zero counts live in [`HotAgentRegistrarError::RuntimeSync`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HotAgentRuntimeSyncOutcome {
    pub registered: usize,
    pub replaced: usize,
    pub failed: usize,
    /// Rows reconciled away: previously-registered `<agent>.*`
    /// abilities whose backing manifest is gone. The registrar owns
    /// the whole `<agent>.` LocalRuntime namespace (see
    /// `unregister_agent`'s prefix wipe), so anything it did not
    /// just (re-)register is stale by definition.
    pub removed: usize,
}

/// Opaque two-phase removal token. Runtime/catalogue rows are already absent;
/// lifecycle persistence must now remove the durable row and identity before
/// calling `commit_agent_removal` to revoke authority.
#[derive(Debug, Clone)]
pub struct HotAgentUnregistration {
    name: String,
    enrollment: crate::daemon::ability::dispatch::HotAgentAuthorityEnrollment,
    outcome: HotAgentRuntimeSyncOutcome,
}

impl HotAgentUnregistration {
    #[must_use]
    pub fn outcome(&self) -> HotAgentRuntimeSyncOutcome {
        self.outcome
    }

    #[must_use]
    pub fn agent_ura(&self) -> &str {
        self.enrollment.authority_root()
    }
}

/// Input for a hot hosted-agent advertise pass.
///
/// `agent_ura` drives `federation.advertise_agent` (identity). When
/// `abilities_payload` + `abilities_resource_ura` are present, the
/// advertiser ALSO fires `federation.advertise_abilities` on the same
/// transport so a hot ability add/remove reaches the hub immediately
/// instead of waiting for the next heartbeat. ISS-002.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotAgentAdvertiseRequest {
    pub agent_ura: String,
    pub generation: u64,
    /// Pre-encoded `federation.advertise_abilities` args (built from the
    /// just-persisted owner projection via
    /// `advertise::advertise_abilities_payload`). `None` skips the
    /// abilities advertise (identity-only). The advertiser targets the
    /// hub federation surface by ability name, so no resource URA is
    /// carried here.
    pub abilities_payload: Option<Vec<u8>>,
}

/// Projection-only publication. Purge tombstones must use this surface so an
/// empty ability set can never re-advertise an identity being deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotAgentProjectionRequest {
    pub agent_ura: String,
    pub generation: u64,
    pub transaction_id: String,
    pub delivery_fence: u64,
    pub abilities_payload: Vec<u8>,
}

/// Outcome for best-effort hub advertisement after hot agent add.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotAgentAdvertiseState {
    Succeeded,
    Failed,
}

/// A configured advertiser always reports an explicit terminal state. Absence
/// of an advertiser remains represented by `Option::None` at the lifecycle
/// boundary and is surfaced as `not_configured` in public responses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotAgentAdvertiseOutcome {
    state: HotAgentAdvertiseState,
    error: Option<String>,
}

impl HotAgentAdvertiseOutcome {
    #[must_use]
    pub fn succeeded() -> Self {
        Self {
            state: HotAgentAdvertiseState::Succeeded,
            error: None,
        }
    }

    #[must_use]
    pub fn failed(error: impl Into<String>) -> Self {
        Self {
            state: HotAgentAdvertiseState::Failed,
            error: Some(error.into()),
        }
    }

    #[must_use]
    pub fn advertised(&self) -> bool {
        self.state == HotAgentAdvertiseState::Succeeded
    }

    #[must_use]
    pub fn state(&self) -> HotAgentAdvertiseState {
        self.state
    }

    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

/// Input for a hot hosted-agent revoke pass (`agent.stop`). Drives
/// `federation.revoke` so the agent identity is removed from the hub
/// directory immediately, symmetric to `advertise_hosted_agent`.
/// ISS-002.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotAgentRevokeRequest {
    pub agent_ura: String,
    pub generation: u64,
    pub reason: String,
    pub purge_transaction_id: Option<String>,
    pub authority_ura: String,
    pub protocol_version: u32,
    pub delivery_fence: u64,
}

/// Narrow abstraction over the transport used to notify the hub
/// about a hot-added hosted agent.
///
/// The registrar owns the trait object so runtime lifecycle code
/// does not depend on `daemon::invocation` concrete
/// session types. Device-mode boot supplies an implementation backed
/// by the current `session.open` bidi; tests can supply a recorder.
pub trait HotAgentAdvertiser: Send + Sync {
    fn advertise_hosted_agent(&self, request: HotAgentAdvertiseRequest)
        -> HotAgentAdvertiseOutcome;

    fn publish_owner_projection(
        &self,
        request: HotAgentProjectionRequest,
    ) -> HotAgentAdvertiseOutcome;

    /// Revoke a hot-removed hosted agent's identity from the hub directory.
    /// Implementations must report success or failure; a default no-op would
    /// turn "not implemented" into fake success.
    fn revoke_hosted_agent(&self, request: HotAgentRevokeRequest) -> HotAgentAdvertiseOutcome;
}

fn ensure_admission_action(
    manifest: crate::daemon::ability::manifest::AbilityManifest,
    default_action: crate::daemon::ability::descriptors::AdmissionAction,
) -> anyhow::Result<crate::daemon::ability::manifest::AbilityManifest> {
    if manifest.admission_action().is_some() {
        Ok(manifest)
    } else {
        manifest.with_admission_action(default_action.as_str())
    }
}

impl HotAgentRegistrar {
    /// Build a *pending* registrar — runtime not yet attached.
    /// Construct at registry-build time so the lifecycle ability
    /// closure can capture a stable `Arc<Self>` before
    /// `LocalRuntime` is built.
    #[must_use]
    pub fn new_pending(
        loaders: Arc<Vec<Arc<dyn ContextLoader>>>,
        dispatch_handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>>,
        discover_federation_resolver: crate::daemon::ability::builtins::agents::discover::SharedDiscoverFederationResolver,
    ) -> Arc<Self> {
        Arc::new(Self {
            runtime: OnceLock::new(),
            loaders,
            dispatch_handle,
            discover_federation_resolver,
            hot_advertiser: OnceLock::new(),
        })
    }

    /// Attach the live `LocalRuntime`. A second writer is a boot wiring error,
    /// not an idempotent success, because handler closures would remain pinned
    /// to the first runtime.
    pub fn set_runtime(&self, runtime: Arc<LocalRuntime>) -> Result<(), HotAgentRegistrarError> {
        self.runtime
            .set(runtime)
            .map_err(|_| HotAgentRegistrarError::SecondWriter {
                dependency: "runtime",
            })
    }

    /// Attach the hot advertise bridge after the device-mode session
    /// escalation handle exists. A second writer is rejected explicitly.
    pub fn set_hot_agent_advertiser(
        &self,
        advertiser: Arc<dyn HotAgentAdvertiser>,
    ) -> Result<(), HotAgentRegistrarError> {
        self.hot_advertiser
            .set(advertiser)
            .map_err(|_| HotAgentRegistrarError::SecondWriter {
                dependency: "hub advertiser",
            })
    }

    /// Clone the current hot advertise bridge if boot wired one.
    #[must_use]
    pub fn hot_agent_advertiser(&self) -> Option<Arc<dyn HotAgentAdvertiser>> {
        self.hot_advertiser.get().cloned()
    }

    #[must_use]
    pub fn readiness(&self) -> HotAgentRegistrarReadiness {
        if self.runtime.get().is_none() {
            HotAgentRegistrarReadiness::PendingRuntime
        } else if self.dispatch_handle.get().is_none() {
            HotAgentRegistrarReadiness::PendingCatalog
        } else {
            HotAgentRegistrarReadiness::Ready
        }
    }

    pub fn require_ready(&self) -> Result<(), HotAgentRegistrarError> {
        let readiness = self.readiness();
        if readiness == HotAgentRegistrarReadiness::Ready {
            Ok(())
        } else {
            Err(HotAgentRegistrarError::NotReady { readiness })
        }
    }

    pub(crate) fn publication_snapshot(
        &self,
    ) -> Result<
        crate::daemon::ability::catalog::LocalAbilityPublicationSnapshot,
        HotAgentRegistrarError,
    > {
        self.require_ready()?;
        let catalog = self
            .dispatch_handle
            .get()
            .expect("Ready registrar must own AxonAbilityCatalog");
        Ok(
            crate::daemon::ability::catalog::LocalAbilityPublicationSnapshot::capture(
                catalog.as_ref(),
            ),
        )
    }

    /// Register the canonical `<agent>.chat / discover / invoke`
    /// triple plus every executable TOML-declared `<agent>.<verb>`
    /// for `name` through the dynamic catalogue transaction.
    ///
    /// **Replace-capable.** Dynamic registration is idempotent at
    /// the product ability name: existing catalogue/control-plane/
    /// runtime rows are replaced as one unit. This is required for
    /// `agent set`, `agent.refresh`, and `meta.acquire`/`forget`,
    /// which update durable state first and then refresh live rows
    /// for names that may already be present.
    pub async fn register_agent(
        &self,
        name: &str,
        entry: &AgentEntry,
    ) -> Result<HotAgentRuntimeSyncOutcome, HotAgentRegistrarError> {
        self.register_agent_replacing(name, entry, None).await
    }

    /// Register a lifecycle replacement with the prior durable entry available
    /// for compensation. If any new row fails, the registrar re-materializes
    /// the prior handler set before returning the typed failure.
    pub async fn register_agent_replacing(
        &self,
        name: &str,
        entry: &AgentEntry,
        previous: Option<&AgentEntry>,
    ) -> Result<HotAgentRuntimeSyncOutcome, HotAgentRegistrarError> {
        if name_claims_reserved_device_owner(name.trim()) {
            crate::op_event!(
                component = axon_bridge,
                kind = hot_agent_register_reserved_owner_rejected,
                agent = name.trim(),
                message = "`device.` is the reserved owner token for \
                          device-sponsored System Agents (RFC-005 §3.1.2); \
                          hosted user agents cannot register under it",
            );
            return Err(HotAgentRegistrarError::ReservedOwner {
                agent: name.trim().to_string(),
            });
        }
        let surface_name = hot_agent_runtime_surface_name(name).map_err(|reason| {
            HotAgentRegistrarError::InvalidAgentIdentifier {
                agent: name.to_string(),
                reason,
            }
        })?;
        let name = surface_name.as_str();
        // DEC-F048 enforcement gate: the registrar owns hosted
        // agents' owner-local public namespace before mapping it to
        // LocalRuntime Ability URA keys. It refuses to mint rows
        // under the reserved `device.` owner token regardless of
        // caller — the lifecycle surface rejects earlier with a
        // user-facing error; this is the invariant's home.
        if name_claims_reserved_device_owner(name) {
            crate::op_event!(
                component = axon_bridge,
                kind = hot_agent_register_reserved_owner_rejected,
                agent = name,
                message = "`device.` is the reserved owner token for \
                          device-sponsored System Agents (RFC-005 §3.1.2); \
                          hosted user agents cannot register under it",
            );
            return Err(HotAgentRegistrarError::ReservedOwner {
                agent: name.to_string(),
            });
        }

        self.require_ready()?;
        let runtime = self
            .runtime
            .get()
            .expect("Ready registrar must own LocalRuntime");
        let catalog = self
            .dispatch_handle
            .get()
            .cloned()
            .expect("Ready registrar must own AxonAbilityCatalog");
        let enrollment = catalog
            .enroll_persisted_hot_agent_authority(name)
            .map_err(|error| HotAgentRegistrarError::AuthorityEnrollment {
                agent: name.to_string(),
                reason: error.to_string(),
            })?;
        let binding = HostedAgentRuntimeBinding {
            agent_ura: enrollment.authority_root().to_string(),
        };

        let authority_scope = match binding.authority_scope(name) {
            Ok(authority_scope) => authority_scope,
            Err(err) => {
                let err_msg = err.to_string();
                crate::op_event!(
                    component = axon_bridge,
                    kind = hot_agent_register_authority_invalid,
                    agent = name,
                    agent_ura = binding.agent_ura.as_str(),
                    error = err_msg.as_str(),
                    message =
                        "hosted agent runtime registration requires a canonical authority scope",
                );
                if previous.is_none() {
                    let _ = catalog.rollback_hot_agent_authority_enrollment(&enrollment);
                }
                return Err(HotAgentRegistrarError::AuthorityScope {
                    agent: name.to_string(),
                    reason: err_msg,
                });
            }
        };

        let mut outcome = HotAgentRuntimeSyncOutcome::default();
        let mut failures = Vec::new();
        let owner = OwnerKind::Agent(name.to_string());
        // Every catalogue row this sync (re-)registers; the reconcile pass at
        // the end removes any other row whose decoded public ability name is
        // `<name>.*` — its backing manifest is gone.
        let mut synced: std::collections::HashSet<String> = std::collections::HashSet::new();
        {
            let mut sync_ctx = HotAgentRuntimeSyncContext {
                runtime,
                catalog: &catalog,
                binding: &binding,
                authority_scope,
                outcome: &mut outcome,
                failures: &mut failures,
            };

            // ── chat
            let chat_ability = format!("{name}.chat");
            let chat_handler =
                build_chat_handler_for(name.to_string(), entry.clone(), Arc::clone(&self.loaders));
            if Self::register_rpc_with_spec(
                &mut sync_ctx,
                &chat_ability,
                owner.clone(),
                crate::daemon::ability::manifest::default_chat_manifest(),
                crate::daemon::ability::descriptors::AdmissionAction::Invoke,
                chat_handler,
            )
            .await
            {
                synced.insert(chat_ability.clone());
            }

            let chat_stream_handler = build_chat_stream_handler_for(
                name.to_string(),
                entry.clone(),
                Arc::clone(&self.loaders),
            );
            if Self::register_stream_with_spec(
                &mut sync_ctx,
                &chat_ability,
                owner.clone(),
                crate::daemon::ability::manifest::default_chat_manifest(),
                crate::daemon::ability::descriptors::AdmissionAction::Invoke,
                chat_stream_handler,
            )
            .await
            {
                synced.insert(chat_ability.clone());
            }

            // ── discover
            let discover_handler = build_discover_handler_for(
                name.to_string(),
                Arc::clone(&self.dispatch_handle),
                Arc::clone(&self.discover_federation_resolver),
            );
            let discover_ability = format!("{name}.discover");
            if Self::register_rpc_with_spec(
                &mut sync_ctx,
                &discover_ability,
                owner.clone(),
                crate::daemon::ability::builtins::agents::discover::manifest(),
                crate::daemon::ability::descriptors::AdmissionAction::Read,
                discover_handler,
            )
            .await
            {
                synced.insert(discover_ability);
            }

            // ── TOML-declared executor-bound abilities. Manifests without
            // `[exec]` are discoverable declarations, not invocable runtime
            // handlers.
            let chat_name = format!("{name}.chat");
            let manifests =
                crate::daemon::execution::mission::agent_ability_specs::manifests_for(name, entry);
            for spec in
                crate::daemon::execution::mission::agent_ability_specs::abilities_for(name, entry)
            {
                let ability_name = spec.name().to_string();
                if ability_name == chat_name {
                    continue;
                }
                let bare = ability_name
                    .strip_prefix(&format!("{name}."))
                    .unwrap_or(&ability_name)
                    .to_string();

                let Some(manifest) = manifests.iter().find(|m| m.name() == bare) else {
                    continue;
                };
                let Some(exec) = manifest.exec() else {
                    continue;
                };
                match exec {
                    crate::daemon::ability::manifest::AbilityExec::HostStream(stream_spec) => {
                        let h = build_host_stream_handler(stream_spec.clone());
                        if Self::register_stream_with_envelope_and_spec(
                            &mut sync_ctx,
                            &ability_name,
                            owner.clone(),
                            manifest.clone(),
                            crate::daemon::ability::descriptors::AdmissionAction::Stream,
                            h,
                        )
                        .await
                        {
                            synced.insert(ability_name);
                        }
                    }
                    _ => {
                        let h = build_agent_ability_handler(
                            name.to_string(),
                            entry.clone(),
                            Arc::clone(&self.loaders),
                            bare,
                        );
                        if Self::register_rpc_with_envelope_and_spec(
                            &mut sync_ctx,
                            &ability_name,
                            owner.clone(),
                            manifest.clone(),
                            crate::daemon::ability::descriptors::AdmissionAction::Invoke,
                            h,
                        )
                        .await
                        {
                            synced.insert(ability_name);
                        }
                    }
                }
            }
        }

        // ── reconcile: a provider withdraws an ability by deleting
        // its TOML; the row must leave the live runtime on the next
        // sync, not on the next daemon restart.
        for stale in hosted_agent_runtime_ability_uras_for_agent(runtime, name).await {
            let Some(dispatch_key) = dispatch_key_for_hosted_agent_runtime_key(&stale, name) else {
                continue;
            };
            if synced.contains(&dispatch_key) {
                continue;
            }
            match catalog.hot_unregister(&dispatch_key) {
                Ok(true) => {
                    outcome.removed += 1;
                    crate::op_event!(
                        component = axon_bridge,
                        kind = hot_agent_ability_reconciled_removed,
                        agent = name,
                        ability = dispatch_key.as_str(),
                        message = "ability manifest gone; dynamic catalogue row removed",
                    );
                }
                Ok(false) => {}
                Err(err) => {
                    outcome.failed += 1;
                    let err_msg = err.to_string();
                    failures.push(HotAgentRuntimeSyncFailure {
                        ability: dispatch_key.clone(),
                        reason: err_msg.clone(),
                    });
                    crate::op_event!(
                        component = axon_bridge,
                        kind = hot_agent_ability_reconcile_failed,
                        agent = name,
                        ability = dispatch_key.as_str(),
                        error = err_msg.as_str(),
                    );
                }
            }
        }

        if failures.is_empty() {
            return Ok(outcome);
        }

        let rollback = if let Some(previous) = previous {
            self.restore_agent_rows(name, previous, runtime, &catalog, &binding)
                .await
        } else {
            self.remove_agent_rows(runtime, &catalog, name).await
        };
        let mut rollback_failures = rollback.err().into_iter().collect::<Vec<_>>();
        if previous.is_none() {
            if let Err(error) = catalog.rollback_hot_agent_authority_enrollment(&enrollment) {
                rollback_failures.push(format!("rollback authority enrollment: {error}"));
            }
        }
        let rollback = if rollback_failures.is_empty() {
            HotAgentRollbackStatus::Completed
        } else {
            HotAgentRollbackStatus::Partial {
                failures: rollback_failures,
            }
        };
        Err(HotAgentRegistrarError::RuntimeSync {
            operation: HotAgentSyncOperation::Register,
            agent: name.to_string(),
            failure_count: failures.len(),
            outcome,
            failures,
            rollback,
        })
    }

    /// Unregister every dynamic `<name>.*` hosted-agent ability.
    /// Returns the count of catalogue rows actually removed.
    ///
    /// Runtime rows are keyed by hosted-agent Ability URAs. Removal
    /// decodes each row back to its owner-local public name before
    /// matching the `<name>.*` product namespace, then removes it
    /// through `AxonAbilityCatalog::hot_unregister` so control-plane
    /// and dynamic side tables cannot drift from the executable row.
    ///
    pub async fn unregister_agent(
        &self,
        name: &str,
        entry: &AgentEntry,
    ) -> Result<HotAgentUnregistration, HotAgentRegistrarError> {
        self.require_ready()?;
        let runtime = self
            .runtime
            .get()
            .expect("Ready registrar must own LocalRuntime");
        let catalog = self
            .dispatch_handle
            .get()
            .cloned()
            .expect("Ready registrar must own AxonAbilityCatalog");
        let enrollment = catalog
            .enroll_persisted_hot_agent_authority(name)
            .map_err(|error| HotAgentRegistrarError::AuthorityEnrollment {
                agent: name.to_string(),
                reason: error.to_string(),
            })?;
        let binding = HostedAgentRuntimeBinding {
            agent_ura: enrollment.authority_root().to_string(),
        };
        let mut outcome = HotAgentRuntimeSyncOutcome::default();
        let mut failures = Vec::new();
        for runtime_key in hosted_agent_runtime_ability_uras_for_agent(runtime, name).await {
            let Some(dispatch_key) = dispatch_key_for_hosted_agent_runtime_key(&runtime_key, name)
            else {
                continue;
            };
            match catalog.hot_unregister(&dispatch_key) {
                Ok(true) => outcome.removed += 1,
                Ok(false) => {}
                Err(err) => {
                    let err_msg = err.to_string();
                    outcome.failed += 1;
                    failures.push(HotAgentRuntimeSyncFailure {
                        ability: dispatch_key.clone(),
                        reason: err_msg.clone(),
                    });
                    crate::op_event!(
                        component = axon_bridge,
                        kind = hot_agent_unregister_failed,
                        agent = name,
                        ability = dispatch_key.as_str(),
                        error = err_msg.as_str(),
                    );
                }
            }
        }
        if failures.is_empty() {
            return Ok(HotAgentUnregistration {
                name: name.to_string(),
                enrollment,
                outcome,
            });
        }

        let rollback = match self
            .restore_agent_rows(name, entry, runtime, &catalog, &binding)
            .await
        {
            Ok(()) => HotAgentRollbackStatus::Completed,
            Err(error) => HotAgentRollbackStatus::Partial {
                failures: vec![error],
            },
        };
        Err(HotAgentRegistrarError::RuntimeSync {
            operation: HotAgentSyncOperation::Unregister,
            agent: name.to_string(),
            failure_count: failures.len(),
            outcome,
            failures,
            rollback,
        })
    }

    /// Commit phase two of `agent.stop`: the lifecycle stores are already
    /// absent, so the opaque removal token may revoke the authority root.
    pub fn commit_agent_removal(
        &self,
        removal: &HotAgentUnregistration,
    ) -> Result<(), HotAgentRegistrarError> {
        let catalog =
            self.dispatch_handle
                .get()
                .cloned()
                .ok_or(HotAgentRegistrarError::NotReady {
                    readiness: HotAgentRegistrarReadiness::PendingCatalog,
                })?;
        catalog
            .revoke_removed_hot_agent_authority(&removal.enrollment)
            .map_err(|error| HotAgentRegistrarError::AuthorityRevocation {
                agent: removal.name.clone(),
                reason: error.to_string(),
            })
    }

    async fn restore_agent_rows(
        &self,
        name: &str,
        entry: &AgentEntry,
        _runtime: &Arc<LocalRuntime>,
        _catalog: &Arc<AxonAbilityCatalog>,
        _binding: &HostedAgentRuntimeBinding,
    ) -> Result<(), String> {
        // Box the compensating call because it reuses the same registration
        // state machine; boxing makes the intentional async recursion finite.
        match Box::pin(self.register_agent_replacing(name, entry, None)).await {
            Ok(_) => Ok(()),
            Err(error) => Err(format!("restore prior hosted-Agent rows: {error}")),
        }
    }

    async fn remove_agent_rows(
        &self,
        runtime: &Arc<LocalRuntime>,
        catalog: &Arc<AxonAbilityCatalog>,
        name: &str,
    ) -> Result<(), String> {
        let mut failures = Vec::new();
        for runtime_key in hosted_agent_runtime_ability_uras_for_agent(runtime, name).await {
            let Some(dispatch_key) = dispatch_key_for_hosted_agent_runtime_key(&runtime_key, name)
            else {
                continue;
            };
            if let Err(error) = catalog.hot_unregister(&dispatch_key) {
                failures.push(format!("{dispatch_key}: {error}"));
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "remove partially registered hosted-Agent rows: {}",
                failures.join("; ")
            ))
        }
    }

    async fn register_rpc_with_spec(
        ctx: &mut HotAgentRuntimeSyncContext<'_>,
        ability_name: &str,
        owner: OwnerKind,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        default_action: crate::daemon::ability::descriptors::AdmissionAction,
        handler: crate::daemon::ability::dispatch::LocalRpcHandler,
    ) -> bool {
        let was_present = match ctx.binding.runtime_ability_ura(ability_name) {
            Some(runtime_key) => {
                Self::runtime_has_mode(
                    ctx.runtime,
                    &runtime_key,
                    crate::daemon::ability::CallMode::Rpc,
                )
                .await
            }
            None => {
                Self::record_bad_runtime_key(ctx.binding, ability_name, ctx.outcome, ctx.failures);
                return false;
            }
        };
        let manifest = match ensure_admission_action(manifest, default_action) {
            Ok(manifest) => manifest,
            Err(error) => {
                Self::record_registration_error(ability_name, error, ctx.outcome, ctx.failures);
                return false;
            }
        };
        match ctx.catalog.hot_register_rpc_with_spec_and_authority_scope(
            ability_name,
            owner,
            ctx.authority_scope.clone(),
            manifest,
            handler,
        ) {
            Ok(()) if was_present => {
                ctx.outcome.replaced += 1;
                true
            }
            Ok(()) => {
                ctx.outcome.registered += 1;
                true
            }
            Err(err) => {
                Self::record_registration_error(ability_name, err, ctx.outcome, ctx.failures);
                false
            }
        }
    }

    async fn register_rpc_with_envelope_and_spec(
        ctx: &mut HotAgentRuntimeSyncContext<'_>,
        ability_name: &str,
        owner: OwnerKind,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        default_action: crate::daemon::ability::descriptors::AdmissionAction,
        handler: crate::daemon::ability::dispatch::LocalRpcHandlerWithEnvelope,
    ) -> bool {
        let was_present = match ctx.binding.runtime_ability_ura(ability_name) {
            Some(runtime_key) => {
                Self::runtime_has_mode(
                    ctx.runtime,
                    &runtime_key,
                    crate::daemon::ability::CallMode::Rpc,
                )
                .await
            }
            None => {
                Self::record_bad_runtime_key(ctx.binding, ability_name, ctx.outcome, ctx.failures);
                return false;
            }
        };
        let manifest = match ensure_admission_action(manifest, default_action) {
            Ok(manifest) => manifest,
            Err(error) => {
                Self::record_registration_error(ability_name, error, ctx.outcome, ctx.failures);
                return false;
            }
        };
        match ctx
            .catalog
            .hot_register_rpc_with_envelope_spec_and_authority_scope(
                ability_name,
                owner,
                ctx.authority_scope.clone(),
                manifest,
                handler,
            ) {
            Ok(()) if was_present => {
                ctx.outcome.replaced += 1;
                true
            }
            Ok(()) => {
                ctx.outcome.registered += 1;
                true
            }
            Err(error) => {
                Self::record_registration_error(ability_name, error, ctx.outcome, ctx.failures);
                false
            }
        }
    }

    async fn register_stream_with_spec(
        ctx: &mut HotAgentRuntimeSyncContext<'_>,
        ability_name: &str,
        owner: OwnerKind,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        default_action: crate::daemon::ability::descriptors::AdmissionAction,
        handler: crate::daemon::ability::dispatch::LocalStreamHandler,
    ) -> bool {
        let was_present = match ctx.binding.runtime_ability_ura(ability_name) {
            Some(runtime_key) => {
                Self::runtime_has_mode(
                    ctx.runtime,
                    &runtime_key,
                    crate::daemon::ability::CallMode::Stream,
                )
                .await
            }
            None => {
                Self::record_bad_runtime_key(ctx.binding, ability_name, ctx.outcome, ctx.failures);
                return false;
            }
        };
        let manifest = match ensure_admission_action(manifest, default_action) {
            Ok(manifest) => manifest,
            Err(error) => {
                Self::record_registration_error(ability_name, error, ctx.outcome, ctx.failures);
                return false;
            }
        };
        match ctx
            .catalog
            .hot_register_stream_with_spec_and_authority_scope(
                ability_name,
                owner,
                ctx.authority_scope.clone(),
                manifest,
                handler,
            ) {
            Ok(()) if was_present => {
                ctx.outcome.replaced += 1;
                true
            }
            Ok(()) => {
                ctx.outcome.registered += 1;
                true
            }
            Err(err) => {
                Self::record_registration_error(ability_name, err, ctx.outcome, ctx.failures);
                false
            }
        }
    }

    async fn register_stream_with_envelope_and_spec(
        ctx: &mut HotAgentRuntimeSyncContext<'_>,
        ability_name: &str,
        owner: OwnerKind,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        default_action: crate::daemon::ability::descriptors::AdmissionAction,
        handler: crate::daemon::ability::dispatch::LocalStreamHandlerWithEnvelope,
    ) -> bool {
        let was_present = match ctx.binding.runtime_ability_ura(ability_name) {
            Some(runtime_key) => {
                Self::runtime_has_mode(
                    ctx.runtime,
                    &runtime_key,
                    crate::daemon::ability::CallMode::Stream,
                )
                .await
            }
            None => {
                Self::record_bad_runtime_key(ctx.binding, ability_name, ctx.outcome, ctx.failures);
                return false;
            }
        };
        let manifest = match ensure_admission_action(manifest, default_action) {
            Ok(manifest) => manifest,
            Err(error) => {
                Self::record_registration_error(ability_name, error, ctx.outcome, ctx.failures);
                return false;
            }
        };
        match ctx
            .catalog
            .hot_register_stream_with_envelope_and_spec_and_authority_scope(
                ability_name,
                owner,
                ctx.authority_scope.clone(),
                manifest,
                handler,
            ) {
            Ok(()) if was_present => {
                ctx.outcome.replaced += 1;
                true
            }
            Ok(()) => {
                ctx.outcome.registered += 1;
                true
            }
            Err(err) => {
                Self::record_registration_error(ability_name, err, ctx.outcome, ctx.failures);
                false
            }
        }
    }

    fn record_bad_runtime_key(
        binding: &HostedAgentRuntimeBinding,
        ability_name: &str,
        outcome: &mut HotAgentRuntimeSyncOutcome,
        failures: &mut Vec<HotAgentRuntimeSyncFailure>,
    ) {
        outcome.failed += 1;
        failures.push(HotAgentRuntimeSyncFailure {
            ability: ability_name.to_string(),
            reason: "derive hosted Agent Ability URA failed".to_string(),
        });
        crate::op_event!(
            component = axon_bridge,
            kind = hot_agent_register_failed,
            ability = ability_name,
            agent_ura = binding.agent_ura.as_str(),
            error = "derive hosted agent ability URA failed",
        );
    }

    async fn runtime_has_mode(
        runtime: &Arc<LocalRuntime>,
        runtime_key: &str,
        call_mode: crate::daemon::ability::CallMode,
    ) -> bool {
        let Some(descriptor) = runtime.ability_descriptor(runtime_key).await else {
            return false;
        };
        match call_mode {
            crate::daemon::ability::CallMode::Rpc => descriptor.options.modes.rpc,
            crate::daemon::ability::CallMode::Stream => descriptor.options.modes.stream,
            crate::daemon::ability::CallMode::Bidi => descriptor.options.modes.bidi,
        }
    }

    fn record_registration_error(
        ability_name: &str,
        err: anyhow::Error,
        outcome: &mut HotAgentRuntimeSyncOutcome,
        failures: &mut Vec<HotAgentRuntimeSyncFailure>,
    ) {
        outcome.failed += 1;
        let err_msg = format!("{err}");
        failures.push(HotAgentRuntimeSyncFailure {
            ability: ability_name.to_string(),
            reason: err_msg.clone(),
        });
        crate::op_event!(
            component = axon_bridge,
            kind = hot_agent_register_failed,
            ability = ability_name,
            error = err_msg.as_str(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::daemon::persistence::agent_registry::{AgentEntry, AgentType};

    fn seed_hosted_agent(name: &str) -> String {
        crate::daemon::persistence::config::save_credentials(
            &crate::daemon::persistence::config::Credentials {
                node_id: "dev".to_string(),
                credential_token: "token".to_string(),
                hub_endpoint: "axon://hub.test:50051".to_string(),
                realm: "localhost".to_string(),
                username: Some("dev".to_string()),
                user_id: Some("user-dev".to_string()),
                ..Default::default()
            },
        )
        .expect("seed paired credentials");
        let host_device_ura = crate::core::ura::device_ura("localhost", "dev");
        let agent_ura = crate::core::ura::agent_ura("localhost", "dev", name);
        crate::daemon::persistence::local_agents::save(
            &crate::daemon::persistence::local_agents::LocalAgentsFile {
                host_device_agent_ura: host_device_ura.clone(),
                hosted_agents: vec![crate::daemon::persistence::local_agents::HostedAgentEntry {
                    profile: "llm".to_string(),
                    name: name.to_string(),
                    agent_ura: agent_ura.clone(),
                    signing_authority: format!("hosted_by:{host_device_ura}"),
                    first_seen_at: "2026-06-24T00:00:00Z".to_string(),
                }],
            },
        )
        .expect("seed local-agents.json");
        let mut registry = crate::daemon::persistence::agent_registry::AgentRegistry::default();
        registry.agents.insert(
            crate::core::agent::id::AgentId::parse(name)
                .expect("test agent name")
                .to_string(),
            AgentEntry::new(AgentType::ClaudeCode, None),
        );
        crate::daemon::persistence::agent_registry::save_agents(&registry)
            .expect("seed durable Agent registry");
        agent_ura
    }

    fn runtime_key(agent: &str, registry_ability: &str) -> String {
        let agent_ura = crate::core::ura::agent_ura("localhost", "dev", agent);
        let public_name = crate::core::ura::owner_local_ability_name(&agent_ura, registry_ability);
        crate::core::ura::owner_ability_ura(&agent_ura, &public_name).expect("runtime key")
    }

    fn build_pending() -> Arc<HotAgentRegistrar> {
        HotAgentRegistrar::new_pending(
            Arc::new(Vec::new()),
            Arc::new(OnceLock::new()),
            Arc::new(crate::daemon::ability::builtins::agents::discover::DetachedDiscoverFederationResolver),
        )
    }

    fn wire_runtime_and_catalog(
        registrar: &Arc<HotAgentRegistrar>,
        runtime: Arc<LocalRuntime>,
    ) -> Arc<AxonAbilityCatalog> {
        registrar
            .set_runtime(Arc::clone(&runtime))
            .expect("test runtime wired once");
        let authority_context = crate::daemon::ability::dispatch::AbilityAuthorityContext::for_device_authority_root_with_hosted_agents(
            crate::core::ura::device_ura("localhost", "dev"),
            Vec::<String>::new(),
        )
        .expect("test Device authority context");
        let catalog = Arc::new(AxonAbilityCatalog::new_with_runtime_and_authority_context(
            runtime,
            authority_context,
        ));
        registrar
            .dispatch_handle
            .set(Arc::clone(&catalog))
            .expect("test catalog wired once");
        catalog
    }

    /// Reconcile pin: a `<agent>.*` row whose backing manifest is
    /// gone (registered by an earlier sync) must leave the runtime on
    /// the next `register_agent`, while the rows this sync owns stay.
    /// This is what lets a provider WITHDRAW an ability via
    /// `agent refresh` instead of a daemon restart.
    #[tokio::test]
    async fn register_agent_rejects_reserved_device_owner_token() {
        let registrar = build_pending();
        let rt = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        );
        let _catalog = wire_runtime_and_catalog(&registrar, Arc::clone(&rt));
        let entry = AgentEntry::new(AgentType::ClaudeCode, None);

        // Dotted form — would mint rows that read as device-owned
        // ability shapes (`device.dev-1.sys.chat`).
        let error = registrar
            .register_agent("device.dev-1.sys", &entry)
            .await
            .expect_err("reserved owner must fail closed");
        assert!(matches!(
            error,
            HotAgentRegistrarError::ReservedOwner { .. }
        ));
        assert!(
            rt.list_abilities().await.is_empty(),
            "no device-owned-shaped rows may reach the runtime"
        );

        // Bare reserved token — would collide with the `device.*`
        // system ability namespace.
        let error = registrar
            .register_agent("device", &entry)
            .await
            .expect_err("bare reserved owner must fail closed");
        assert!(matches!(
            error,
            HotAgentRegistrarError::ReservedOwner { .. }
        ));
        assert!(rt.list_abilities().await.is_empty());

        // User-owned shape passes the same gate untouched.
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        seed_hosted_agent("liangbing");
        registrar
            .register_agent("liangbing", &entry)
            .await
            .expect("user-hosted Agent registers");
        assert!(
            rt.has_ability(&runtime_key("liangbing", "liangbing.chat"))
                .await
        );
    }

    #[tokio::test]
    async fn register_agent_reconciles_rows_without_backing_manifests() {
        let registrar = build_pending();
        let rt = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        );
        let catalog = wire_runtime_and_catalog(&registrar, Arc::clone(&rt));
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        seed_hosted_agent("liangbing");

        // Simulate an earlier sync's TOML ability whose manifest has
        // since been deleted: the row exists in the runtime but no
        // current source will re-register it.
        let ghost_key = runtime_key("liangbing", "liangbing.ghost_op");
        catalog
            .enroll_persisted_hot_agent_authority("liangbing")
            .expect("durable hosted-Agent authority enrollment");
        let ghost_authority = HostedAgentRuntimeBinding::load("liangbing")
            .expect("hosted Agent binding")
            .authority_scope("liangbing")
            .expect("hosted Agent authority scope");
        catalog
            .hot_register_rpc_with_spec_and_authority_scope(
                "liangbing.ghost_op",
                OwnerKind::Agent("liangbing".to_string()),
                ghost_authority,
                crate::daemon::ability::manifest::default_chat_manifest()
                    .with_admission_action("invoke")
                    .unwrap(),
                Arc::new(|_args| Ok(serde_json::Value::Null)),
            )
            .expect("seed dynamic ghost ability");
        assert!(rt.has_ability(&ghost_key).await);

        let entry = AgentEntry::new(AgentType::ClaudeCode, None);
        let outcome = registrar
            .register_agent("liangbing", &entry)
            .await
            .expect("reconcile succeeds");

        assert_eq!(outcome.removed, 1, "stale row must be reconciled away");
        assert!(
            !rt.has_ability(&ghost_key).await,
            "withdrawn ability must leave the live runtime"
        );
        assert!(
            rt.has_ability(&runtime_key("liangbing", "liangbing.chat"))
                .await,
            "rows owned by this sync must survive the reconcile"
        );
    }

    #[tokio::test]
    async fn register_agent_lands_chat_discover_invoke_into_runtime_after_set_runtime() {
        // **Phase 5c invariant pin.**
        //
        // After `set_runtime`, calling `register_agent("liangbing", entry)`
        // MUST make `runtime.has_ability("liangbing.chat") == true` —
        // the load-bearing property the dispatcher's Phase-4 arm
        // canonical dispatch and the host's session-receive
        // Axon arm (`LocalAxonSessionDispatcher`) both gate on.
        //
        // Pre-this-PR, `agent.start` only wrote `agents.json`
        // and the hot-added agent surfaced ONLY through the retired
        // lookup-miss catalog path. Chat worked, but every call went
        // through that path, never reaching the wired `LedgerSink` —
        // so `invocations.redb` stayed empty even on successful
        // chats. This test pins the fix at the
        // registrar layer; the boot-side wiring + lifecycle handler
        // wiring are tested separately.
        let registrar = build_pending();
        let rt = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        );
        let _catalog = wire_runtime_and_catalog(&registrar, Arc::clone(&rt));
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        seed_hosted_agent("liangbing");

        let entry = AgentEntry::new(AgentType::ClaudeCode, None);
        let outcome = registrar
            .register_agent("liangbing", &entry)
            .await
            .expect("register succeeds");

        assert!(
            outcome.registered >= 2,
            "chat/discover abilities must land (plus any TOML), got {outcome:?}"
        );
        assert_eq!(outcome.failed, 0);
        assert_eq!(outcome.replaced, 0);

        // The load-bearing checks — these are what the Phase-4 arm
        // and the Phase-5d session-receive arm gate on.
        assert!(
            rt.has_ability(&runtime_key("liangbing", "liangbing.chat"))
                .await,
            "liangbing.chat MUST be in LocalRuntime after register_agent"
        );
        let wrong_device_scoped_key = crate::core::ura::owner_ability_ura(
            &crate::core::ura::device_agent_ura("localhost", "dev", "liangbing"),
            "chat",
        )
        .expect("device-scoped negative key");
        assert!(
            !rt.has_ability(&wrong_device_scoped_key).await,
            "hosted Agent runtime rows must not inherit the host Device authority root"
        );
        assert!(
            rt.has_ability(&runtime_key("liangbing", "liangbing.discover"))
                .await
        );
        assert!(
            !rt.has_ability(&runtime_key("liangbing", "liangbing.invoke"))
                .await
        );
    }

    #[tokio::test]
    async fn register_agent_replaces_existing_runtime_rows_without_duplicate_failures() {
        // `agent set` and `agent.refresh` both call
        // `register_agent` for an agent that may already be live.
        // The runtime sync must replace those rows instead of
        // reporting duplicate-name failures and leaving old handler
        // closures in place.
        use axon_sdk::invocation::AbilityChangeEvent;

        let registrar = build_pending();
        let rt = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        );
        let _catalog = wire_runtime_and_catalog(&registrar, Arc::clone(&rt));
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        seed_hosted_agent("liangbing");

        let first_entry = AgentEntry::new(AgentType::ClaudeCode, Some("sonnet".to_string()));
        let first = registrar
            .register_agent("liangbing", &first_entry)
            .await
            .expect("initial register succeeds");
        assert!(first.registered >= 2, "initial register must land rows");
        assert_eq!(first.replaced, 0);
        assert_eq!(first.failed, 0);

        let mut changes = rt.subscribe_ability_changes();
        let second_entry = AgentEntry::new(AgentType::ClaudeCode, Some("opus".to_string()));
        let second = registrar
            .register_agent("liangbing", &second_entry)
            .await
            .expect("replacement succeeds");
        assert_eq!(
            second.registered, 0,
            "refreshing an existing agent should not create duplicate rows"
        );
        assert!(
            second.replaced >= 2,
            "chat/discover abilities must be replaced, got {second:?}"
        );
        assert_eq!(
            second.failed, 0,
            "duplicate-name failures would leave stale handler closures live"
        );

        let expected_chat = runtime_key("liangbing", "liangbing.chat");
        let expected_discover = runtime_key("liangbing", "liangbing.discover");
        let mut replaced = std::collections::BTreeSet::new();
        for _ in 0..16 {
            let Ok(event) =
                tokio::time::timeout(std::time::Duration::from_millis(100), changes.recv()).await
            else {
                break;
            };
            if let Ok(AbilityChangeEvent::Replaced { name, .. }) = event {
                replaced.insert(name);
            }
            if replaced.contains(&expected_chat) && replaced.contains(&expected_discover) {
                break;
            }
        }
        assert!(
            replaced.contains(&expected_chat) && replaced.contains(&expected_discover),
            "runtime must broadcast replacement for chat/discover, got {replaced:?}"
        );
    }

    #[tokio::test]
    async fn register_agent_before_set_runtime_fails_with_typed_readiness() {
        let registrar = build_pending();
        let entry = AgentEntry::new(AgentType::ClaudeCode, None);
        let error = registrar
            .register_agent("liangbing", &entry)
            .await
            .expect_err("pending registrar must fail closed");
        assert_eq!(
            error,
            HotAgentRegistrarError::NotReady {
                readiness: HotAgentRegistrarReadiness::PendingRuntime,
            }
        );
    }

    #[tokio::test]
    async fn unregister_agent_removes_every_matching_hosted_agent_runtime_key() {
        // The reverse runtime-sync invariant: `agent.stop`
        // must wipe the `<name>.*` public set after decoding
        // LocalRuntime Ability URA keys back to owner-local public
        // names, so `runtime.has_ability` flips back to `false`.
        let registrar = build_pending();
        let rt = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        );
        let _catalog = wire_runtime_and_catalog(&registrar, Arc::clone(&rt));
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        seed_hosted_agent("liangbing");

        let entry = AgentEntry::new(AgentType::ClaudeCode, None);
        registrar
            .register_agent("liangbing", &entry)
            .await
            .expect("register succeeds");
        assert!(
            rt.has_ability(&runtime_key("liangbing", "liangbing.chat"))
                .await
        );

        let removal = registrar
            .unregister_agent("liangbing", &entry)
            .await
            .expect("unregister succeeds");
        let removed = removal.outcome().removed;
        assert!(
            removed >= 2,
            "chat/discover abilities must be removed, got {removed}"
        );
        assert!(
            !rt.has_ability(&runtime_key("liangbing", "liangbing.chat"))
                .await
        );
        assert!(
            !rt.has_ability(&runtime_key("liangbing", "liangbing.discover"))
                .await
        );
        assert!(
            !rt.has_ability(&runtime_key("liangbing", "liangbing.invoke"))
                .await
        );
    }
}
