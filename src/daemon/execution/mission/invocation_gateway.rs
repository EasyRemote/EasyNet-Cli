// EasyNet daemon - Mission child Invocation gateway
// =================================================
//
// Mission orchestration executes inside an admitted Axon Ability. Every
// nested call must therefore be a capability-derived child of that exact
// parent, never a new daemon-local root reconstructed from JSON metadata.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use axon_sdk::invocation::{
    AbilityContext, AgentIdentity, CallMode, ChildInvocationRequest,
    DescriptorBoundInvocationTarget, InvocationState, LocalRuntime, ResourceLimit, SubjectIdentity,
    SupervisorSpec, UraProfile,
};
use serde_json::Value;

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::axon_bridge::local_runtime_request::AXON_TRACE_CONTEXT_METADATA_KEY;
use crate::daemon::persistence::agent_aggregate::{
    AgentAggregateRepository, HostedAgentNameLookupError,
};
/// One child Invocation requested by Mission orchestration.
///
/// Parent identity, subject, cause, trace, deadline ceiling, and cancellation
/// are intentionally absent. They are owned by [`DaemonMissionInvocationGateway`]
/// and derived from its runtime-minted parent capability.
#[derive(Debug, Clone)]
pub(crate) struct MissionInvocationRequest {
    ability: String,
    args: Value,
    target: MissionInvocationTarget,
    timeout: Duration,
    dependency_receipts: Vec<MissionReceiptReference>,
    trace_id: Option<String>,
}

/// Product-neutral execution target for one composite-ability child.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MissionInvocationTarget {
    LocalDevice,
    HostedAgent(String),
    RemoteNode(String),
}

/// Downstream policy capability applied to a runtime-derived Mission child.
///
/// Axon remains the sole owner of canonical admission, lifecycle, and
/// receipts. Implementations stage only product policy that cannot be encoded
/// in the canonical descriptor-bound envelope.
pub(crate) trait MissionChildAdmissionProvider: Send + Sync {
    fn stage_child(
        &self,
        descriptor_bound: &axon_sdk::invocation::DescriptorBoundEnvelope,
        signed_envelope: &axon_sdk::invocation::SignedEnvelope,
        arguments: Vec<u8>,
        metadata: HashMap<String, String>,
        request_id: String,
        ability: &str,
        call_mode: CallMode,
    ) -> anyhow::Result<Box<dyn PendingMissionChildAdmission>>;
}

/// One staged downstream policy transaction.
///
/// Dropping the value rolls the policy reservation back. Committing is only
/// valid after Axon has admitted the child and returned its canonical handle.
pub(crate) trait PendingMissionChildAdmission: Send {
    fn commit(self: Box<Self>) -> anyhow::Result<()>;
}

#[cfg(feature = "axon-pb")]
impl MissionChildAdmissionProvider
    for crate::daemon::invocation::admission::admission_facade::DaemonDerivedInvocationAdmission
{
    fn stage_child(
        &self,
        descriptor_bound: &axon_sdk::invocation::DescriptorBoundEnvelope,
        signed_envelope: &axon_sdk::invocation::SignedEnvelope,
        arguments: Vec<u8>,
        metadata: HashMap<String, String>,
        request_id: String,
        ability: &str,
        call_mode: CallMode,
    ) -> anyhow::Result<Box<dyn PendingMissionChildAdmission>> {
        let lease = self
            .stage(
                descriptor_bound,
                signed_envelope,
                arguments,
                metadata,
                request_id,
                ability,
                call_mode,
            )
            .map_err(|status| anyhow::anyhow!(status.message().to_string()))?;
        Ok(Box::new(lease))
    }
}

#[cfg(feature = "axon-pb")]
impl PendingMissionChildAdmission
    for crate::daemon::invocation::admission::admission_facade::DaemonProductAdmissionLease
{
    fn commit(self: Box<Self>) -> anyhow::Result<()> {
        (*self)
            .commit()
            .map(|_| ())
            .map_err(|status| anyhow::anyhow!(status.message().to_string()))
    }
}

impl MissionInvocationRequest {
    /// Invoke a device-owned system ability.
    pub(crate) fn system(ability: impl Into<String>, args: Value) -> Self {
        Self {
            ability: ability.into(),
            args,
            target: MissionInvocationTarget::LocalDevice,
            timeout: default_child_timeout(),
            dependency_receipts: Vec::new(),
            trace_id: None,
        }
    }

    /// Invoke an ability owned by a locally hosted Agent.
    pub(crate) fn hosted_agent(
        agent: impl Into<String>,
        ability: impl Into<String>,
        args: Value,
    ) -> Self {
        Self {
            ability: ability.into(),
            args,
            target: MissionInvocationTarget::HostedAgent(agent.into()),
            timeout: default_child_timeout(),
            dependency_receipts: Vec::new(),
            trace_id: None,
        }
    }

    pub(crate) fn remote_node(
        target_ura: impl Into<String>,
        ability: impl Into<String>,
        args: Value,
    ) -> anyhow::Result<Self> {
        let target_ura = target_ura.into();
        let target_ura = target_ura.trim();
        let parsed = axon_sdk::ura::parse_ura(target_ura)
            .map_err(|error| anyhow::anyhow!("invalid remote Mission target URA: {error}"))?;
        if !matches!(
            parsed.kind,
            axon_sdk::ura::URAKind::Device | axon_sdk::ura::URAKind::Hub
        ) {
            anyhow::bail!(
                "remote Mission target must be a Device or Hub URA, got {}",
                parsed.kind
            );
        }
        Ok(Self {
            ability: ability.into(),
            args,
            target: MissionInvocationTarget::RemoteNode(target_ura.to_string()),
            timeout: default_child_timeout(),
            dependency_receipts: Vec::new(),
            trace_id: None,
        })
    }

    pub(crate) fn with_dispatch_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub(crate) fn with_dependency_receipts(
        mut self,
        dependency_receipts: Vec<MissionReceiptReference>,
    ) -> Self {
        self.dependency_receipts = dependency_receipts;
        self
    }

    pub(crate) fn with_trace_id(mut self, trace_id: impl Into<String>) -> Self {
        self.trace_id = Some(trace_id.into());
        self
    }

    #[cfg(test)]
    pub(crate) fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    #[cfg(test)]
    pub(crate) fn ability(&self) -> &str {
        &self.ability
    }

    #[cfg(test)]
    pub(crate) fn hosted_agent_name(&self) -> Option<&str> {
        match &self.target {
            MissionInvocationTarget::HostedAgent(agent) => Some(agent),
            _ => None,
        }
    }
}

fn default_child_timeout() -> Duration {
    Duration::from_secs(crate::support::platform::timeouts::INVOKE_DEFAULT_SECS)
}

/// Mission-facing port for daemon-owned canonical Invocation.
pub(crate) trait MissionInvocationGateway: Send + Sync {
    fn invoke(&self, request: MissionInvocationRequest)
        -> anyhow::Result<MissionInvocationOutcome>;
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct MissionReceiptReference {
    invocation_ura: String,
    receipt_ura: String,
    receipt_hash: [u8; 32],
}

impl MissionReceiptReference {
    pub(crate) fn projection(&self) -> Value {
        serde_json::json!({
            "invocation_ura": self.invocation_ura,
            "receipt_ura": self.receipt_ura,
            "receipt_hash": hex::encode(self.receipt_hash),
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MissionInvocationRecord {
    envelope: axon_sdk::invocation::InvocationEnvelope,
    invocation_ura: String,
    terminal_receipt: MissionReceiptReference,
    dependency_receipts: Vec<MissionReceiptReference>,
}

impl MissionInvocationRecord {
    pub(crate) fn terminal_receipt(&self) -> &MissionReceiptReference {
        &self.terminal_receipt
    }

    pub(crate) fn projection(&self) -> Value {
        serde_json::json!({
            "invocation_ura": self.invocation_ura,
            "caller_ura": self.envelope.caller.ura,
            "callee_ura": self.envelope.callee.ura,
            "ability": self.envelope.ability,
            "subject_ura": self.envelope.subject.ura,
            "invocation_nonce": hex::encode(self.envelope.invocation_nonce),
            "causal_context": causal_context_projection(&self.envelope.causal_context),
            "dependency_receipts": self.dependency_receipts.iter()
                .map(MissionReceiptReference::projection)
                .collect::<Vec<_>>(),
            "receipt": {"anchor": self.terminal_receipt.projection()},
        })
    }

    #[cfg(test)]
    pub(crate) fn for_test(ability: &str, marker: u8) -> Self {
        let caller = AgentIdentity::new("easynet:///r/test/device/mission", UraProfile::StrictV2);
        let callee = AgentIdentity::new("easynet:///r/test/agent/worker", UraProfile::StrictV2);
        let subject =
            SubjectIdentity::new("easynet:///r/test/resource/mission", UraProfile::StrictV2);
        let invocation_id = format!("mission-test-{marker:02x}");
        let invocation_ura = axon_sdk::ura::invocation_record_ura_for_binding(
            &subject.ura,
            &callee.ura,
            &caller.ura,
            &invocation_id,
        )
        .expect("test identities own canonical Invocation records");
        let envelope = axon_sdk::invocation::CanonicalEnvelopeBuilder::new(
            caller,
            callee,
            subject,
            axon_sdk::invocation::InvocationDerivationPolicy::Explicit {
                invocation_nonce: [marker; 16],
                causal_context: axon_sdk::invocation::CausalContext::None,
            },
        )
        .and_then(|builder| builder.invocation_envelope(ability, &[marker]))
        .expect("construct canonical Mission test Invocation");
        Self {
            envelope,
            terminal_receipt: MissionReceiptReference {
                invocation_ura: invocation_ura.clone(),
                receipt_ura: format!("{invocation_ura}/receipt/1"),
                receipt_hash: [marker; 32],
            },
            invocation_ura,
            dependency_receipts: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct MissionInvocationOutcome {
    pub(crate) value: Value,
    pub(crate) invocation: MissionInvocationRecord,
}

trait MissionChildTargetResolver: Send + Sync {
    fn callee_ura(&self, request: &MissionInvocationRequest) -> anyhow::Result<String>;
}

#[derive(Debug, Default)]
struct PersistedMissionChildTargetResolver;

impl MissionChildTargetResolver for PersistedMissionChildTargetResolver {
    fn callee_ura(&self, request: &MissionInvocationRequest) -> anyhow::Result<String> {
        let agent_name = match &request.target {
            MissionInvocationTarget::LocalDevice => {
                return Ok(crate::daemon::identity::local_invocation::local_device_ura());
            }
            MissionInvocationTarget::RemoteNode(target) => return Ok(target.clone()),
            MissionInvocationTarget::HostedAgent(agent_name) => agent_name,
        };
        let agent_name = agent_name.trim();
        if agent_name.is_empty() {
            anyhow::bail!("Mission child hosted Agent name must not be empty");
        }
        let snapshot = AgentAggregateRepository::try_load_snapshot()
            .context("load Agent aggregate for Mission child target")?;
        let agent_ura = snapshot
            .hosted_agent_ura_by_name(agent_name)
            .map_err(|error| mission_child_hosted_agent_lookup_error(agent_name, error))?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Mission child hosted Agent {agent_name:?} is not registered in the Agent aggregate"
                )
            })?;
        Ok(agent_ura.to_string())
    }
}

fn mission_child_hosted_agent_lookup_error(
    agent_name: &str,
    error: HostedAgentNameLookupError,
) -> anyhow::Error {
    match error {
        HostedAgentNameLookupError::Ambiguous {
            first_profile,
            second_profile,
            ..
        } => anyhow::anyhow!(
            "Mission child hosted Agent {agent_name:?} is ambiguous across profiles {first_profile:?} and {second_profile:?}"
        ),
        HostedAgentNameLookupError::InvalidUra {
            agent_ura, reason, ..
        } => anyhow::anyhow!(
            "Mission child hosted Agent {agent_name:?} has invalid URA {agent_ura:?}: {reason}"
        ),
        HostedAgentNameLookupError::NonAgentUra { agent_ura, .. } => anyhow::anyhow!(
            "Mission child hosted Agent {agent_name:?} resolved to non-Agent URA {agent_ura:?}"
        ),
    }
}

/// Invocation-scoped production gateway.
///
/// The gateway can only be built from an envelope-aware handler context that
/// still owns Axon's runtime-minted [`AbilityContext`]. That capability is
/// consumed by `prepare_child_dispatch`, which binds the child to the real
/// parent admission receipt and rejects dispatch after parent seal/cancel.
pub(crate) struct DaemonMissionInvocationGateway {
    parent: Arc<AbilityContext>,
    runtime: Arc<LocalRuntime>,
    child_admission: MissionChildAdmission,
    parent_subject: SubjectIdentity,
    trace_id: Option<String>,
    target_resolver: Arc<dyn MissionChildTargetResolver>,
}

enum MissionChildAdmission {
    Daemon(Arc<dyn MissionChildAdmissionProvider>),
    #[cfg(test)]
    CanonicalOnly,
}

impl DaemonMissionInvocationGateway {
    pub(crate) fn from_admitted_envelope(envelope: &EnvelopeContext) -> anyhow::Result<Self> {
        let parent = envelope.runtime_invocation_context().ok_or_else(|| {
            anyhow::anyhow!("Mission child dispatch requires an admitted Axon parent capability")
        })?;
        let runtime = envelope.shared_runtime().ok_or_else(|| {
            anyhow::anyhow!("Mission child dispatch requires the admitting LocalRuntime")
        })?;
        let child_admission = envelope.derived_invocation_admission().ok_or_else(|| {
            anyhow::anyhow!(
                "Mission child dispatch requires the admitting daemon product-policy capability"
            )
        })?;
        Self::from_runtime_context(
            parent,
            runtime,
            MissionChildAdmission::Daemon(child_admission),
            Arc::new(PersistedMissionChildTargetResolver),
        )
    }

    pub(crate) fn from_envelope(
        envelope: &EnvelopeContext,
        runtime: Arc<LocalRuntime>,
    ) -> anyhow::Result<Self> {
        let parent = envelope.runtime_invocation_context().ok_or_else(|| {
            anyhow::anyhow!("Mission child dispatch requires an admitted Axon parent capability")
        })?;
        let admitted_runtime = envelope.shared_runtime().ok_or_else(|| {
            anyhow::anyhow!("Mission child dispatch requires the admitting LocalRuntime")
        })?;
        if !Arc::ptr_eq(&runtime, &admitted_runtime) {
            anyhow::bail!(
                "Mission child dispatch runtime does not match the parent's admitting LocalRuntime"
            );
        }
        let child_admission = envelope.derived_invocation_admission().ok_or_else(|| {
            anyhow::anyhow!(
                "Mission child dispatch requires the admitting daemon product-policy capability"
            )
        })?;
        Self::from_runtime_context(
            parent,
            admitted_runtime,
            MissionChildAdmission::Daemon(child_admission),
            Arc::new(PersistedMissionChildTargetResolver),
        )
    }

    fn from_runtime_context(
        parent: Arc<AbilityContext>,
        runtime: Arc<LocalRuntime>,
        child_admission: MissionChildAdmission,
        target_resolver: Arc<dyn MissionChildTargetResolver>,
    ) -> anyhow::Result<Self> {
        let signed_parent = parent.signed_envelope().ok_or_else(|| {
            anyhow::anyhow!(
                "Mission child dispatch parent {} has no admitted signed envelope",
                parent.invocation_id
            )
        })?;
        let parent_subject = signed_parent.envelope.subject.clone();
        let trace_id = parent
            .request_metadata
            .get(AXON_TRACE_CONTEXT_METADATA_KEY)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        Ok(Self {
            parent,
            runtime,
            child_admission,
            parent_subject,
            trace_id,
            target_resolver,
        })
    }

    async fn invoke_child(
        &self,
        request: MissionInvocationRequest,
    ) -> anyhow::Result<MissionInvocationOutcome> {
        let callee_ura = self.target_resolver.callee_ura(&request)?;
        let MissionInvocationRequest {
            ability,
            args,
            target: execution_target,
            timeout,
            dependency_receipts,
            trace_id,
        } = request;
        let runtime_ability =
            crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(&callee_ura, &ability)
                .map_err(|error| anyhow::anyhow!("resolve Mission child ability URA: {error}"))?;
        let descriptor_ref = match &execution_target {
            MissionInvocationTarget::RemoteNode(_) => {
                crate::daemon::axon_bridge::descriptor_ref::catalog_descriptor_ref_for_wire(
                    &callee_ura,
                    &ability,
                    crate::daemon::ability::CallMode::Rpc,
                )
                .map_err(|error| anyhow::anyhow!("bind remote Mission child descriptor: {error}"))?
            }
            _ => {
                let descriptor_binding =
                    crate::daemon::axon_bridge::descriptor_ref::registered_descriptor_binding(
                        &self.runtime,
                        &runtime_ability,
                        CallMode::Rpc,
                    )
                    .await
                    .map_err(|error| {
                        anyhow::anyhow!("resolve Mission child descriptor: {error}")
                    })?;
                crate::daemon::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
                    &callee_ura,
                    &ability,
                    &descriptor_binding,
                )
                .map_err(|error| anyhow::anyhow!("bind Mission child descriptor: {error}"))?
            }
        };
        let descriptor_target = DescriptorBoundInvocationTarget::new(
            AgentIdentity::new(callee_ura, UraProfile::StrictV2),
            descriptor_ref,
        )
        .map_err(|error| anyhow::anyhow!("construct Mission child target: {error}"))?;
        let payload = serde_json::to_vec(&args).context("encode Mission child arguments")?;
        let child =
            ChildInvocationRequest::new(descriptor_target, self.parent_subject.clone(), payload)
                .with_supervisor(SupervisorSpec {
                    resource_limit: ResourceLimit {
                        wall_seconds: Some(timeout.as_secs_f64()),
                        ..Default::default()
                    },
                    ..Default::default()
                });

        let prepared = self
            .parent
            .prepare_child_dispatch(child)
            .await
            .map_err(|error| anyhow::anyhow!("derive Mission child from parent: {error}"))?;
        let signed_child = prepared.signed_envelope().clone();
        let inherited_deadline = prepared.inherited_absolute_deadline();
        let mut descriptor_request = prepared.into_descriptor_request();
        let trace_id = trace_id.as_deref().or(self.trace_id.as_deref());
        let request_metadata = trace_id
            .map(|trace_id| {
                HashMap::from([(
                    AXON_TRACE_CONTEXT_METADATA_KEY.to_string(),
                    trace_id.to_string(),
                )])
            })
            .unwrap_or_default();
        if let Some(trace_id) = trace_id {
            descriptor_request = descriptor_request.with_trace_id(trace_id);
        }
        descriptor_request = descriptor_request.with_request_metadata(request_metadata.clone());
        let (value, invocation_id, receipt_index, receipt_hash) =
            if matches!(execution_target, MissionInvocationTarget::RemoteNode(_)) {
                self.forward_remote_child(
                    &ability,
                    descriptor_request,
                    &signed_child,
                    inherited_deadline,
                    trace_id,
                )
                .await?
            } else {
                let product_admission = match &self.child_admission {
                    MissionChildAdmission::Daemon(admission) => Some(
                        admission
                            .stage_child(
                                descriptor_request.envelope(),
                                &signed_child,
                                descriptor_request.payload().to_vec(),
                                request_metadata,
                                trace_id.unwrap_or_default().to_string(),
                                &ability,
                                CallMode::Rpc,
                            )
                            .map_err(|error| {
                                anyhow::anyhow!(
                                    "stage Mission child {ability} product admission: {error}"
                                )
                            })?,
                    ),
                    #[cfg(test)]
                    MissionChildAdmission::CanonicalOnly => None,
                };
                let (handle, _) = self
                    .runtime
                    .invoke_descriptor_bound_request_async(descriptor_request)
                    .await
                    .map_err(|error| anyhow::anyhow!("admit Mission child {ability}: {error}"))?;
                if let Some(product_admission) = product_admission {
                    product_admission.commit().map_err(|error| {
                        anyhow::anyhow!("commit Mission child {ability} product admission: {error}")
                    })?;
                }
                let finalized = handle.finalized().await.map_err(|error| {
                    anyhow::anyhow!("finalize Mission child {ability}: {error}")
                })?;
                if finalized.terminal_state != InvocationState::Completed {
                    if let Some(error) = finalized.failure {
                        return Err(anyhow::anyhow!(error))
                            .with_context(|| format!("Mission child {ability} failed"));
                    }
                    anyhow::bail!(
                        "Mission child {ability} ended in {:?}",
                        finalized.terminal_state
                    );
                }
                (
                    decode_child_output(finalized.output(), &ability)?,
                    finalized.terminal_receipt.invocation_id().to_string(),
                    finalized.terminal_receipt.index(),
                    finalized.terminal_receipt.self_hash(),
                )
            };
        let invocation_ura = canonical_invocation_ura(&invocation_id, &signed_child.envelope)?;
        let terminal_receipt = MissionReceiptReference {
            receipt_ura: format!("{invocation_ura}/receipt/{receipt_index}"),
            invocation_ura: invocation_ura.clone(),
            receipt_hash,
        };
        Ok(MissionInvocationOutcome {
            value,
            invocation: MissionInvocationRecord {
                envelope: signed_child.envelope,
                invocation_ura,
                terminal_receipt,
                dependency_receipts,
            },
        })
    }

    #[cfg(feature = "axon-pb")]
    async fn forward_remote_child(
        &self,
        ability: &str,
        descriptor_request: axon_sdk::invocation::DescriptorBoundInvocationRequest,
        signed_child: &axon_sdk::invocation::SignedEnvelope,
        inherited_deadline: Option<tokio::time::Instant>,
        trace_id: Option<&str>,
    ) -> anyhow::Result<(Value, String, u64, [u8; 32])> {
        use axon_sdk::invocation::{project_wire_envelope, WireEnvelopeMetadata};
        use axon_sdk::pb::axon::v1::invocation_client::InvocationClient;
        use axon_sdk::pb::axon::v1::{ContentEnvelope, InvokeRequest};

        let descriptor_ref = signed_child.envelope.ability.clone();
        let remaining_budget = inherited_deadline
            .and_then(|deadline| deadline.checked_duration_since(tokio::time::Instant::now()))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Mission remote child {ability} has no live inherited dispatch deadline"
                )
            })?;
        let deadline_unix_ms = std::time::SystemTime::now()
            .checked_add(remaining_budget)
            .and_then(|deadline| deadline.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|duration| i64::try_from(duration.as_millis()).ok())
            .unwrap_or_default();
        let timeout_seconds = i32::try_from(remaining_budget.as_secs().max(1)).unwrap_or(i32::MAX);
        let envelope = &signed_child.envelope;
        let mut metadata = HashMap::new();
        if let Some(trace_id) = trace_id {
            metadata.insert(
                AXON_TRACE_CONTEXT_METADATA_KEY.to_string(),
                trace_id.to_string(),
            );
        }
        let wire_envelope = project_wire_envelope(
            envelope,
            WireEnvelopeMetadata {
                request_id: uuid::Uuid::new_v4().to_string(),
                deadline_unix_ms,
                trace_id: trace_id.unwrap_or_default().to_string(),
                caller_signature: Some(signed_child.signature.clone()),
                ..WireEnvelopeMetadata::default()
            },
        )
        .map_err(|error| anyhow::anyhow!("project Mission child invocation: {error}"))?;
        let request = InvokeRequest {
            envelope: Some(wire_envelope),
            target: Some(
                crate::daemon::invocation::dispatch::invocation_wire::wire_invocation_target(
                    &descriptor_ref,
                    ability,
                )
                .map_err(|error| anyhow::anyhow!("project Mission child target: {error}"))?,
            ),
            function_name: ability.to_string(),
            arguments: descriptor_request.payload().to_vec(),
            content_type: "application/json".to_string(),
            timeout_seconds,
            metadata,
            content_envelope: Some(ContentEnvelope {
                content_type: "application/json".to_string(),
                encoding: "identity".to_string(),
                ..ContentEnvelope::default()
            }),
            ..InvokeRequest::default()
        };
        let binding =
            crate::daemon::invocation::dispatch::forwarded_finalization::ForwardedInvocationBinding::from_request(&request)
                .map_err(|status| anyhow::anyhow!(
                    "Mission remote child request is not receipt-verifiable: {}",
                    status.message()
                ))?;
        let socket = crate::support::platform::local_daemon_grpc::resolve_socket_path();
        let connect_timeout = remaining_budget.min(Duration::from_secs(10));
        let channel = tokio::select! {
            channel = crate::support::platform::local_daemon_grpc::connect_channel(
                socket,
                remaining_budget,
                connect_timeout,
            ) => channel.context("connect Mission child to canonical daemon Invocation route")?,
            () = self.parent.wait_for_cancel() => {
                anyhow::bail!("Mission parent cancelled while remote child {ability} was connecting")
            }
            () = wait_for_deadline(inherited_deadline) => {
                anyhow::bail!("Mission parent deadline elapsed while remote child {ability} was connecting")
            }
        };
        let mut client = InvocationClient::new(channel);
        let response = tokio::select! {
            response = client.invoke(request) => response
                .map_err(|status| anyhow::anyhow!(
                    "daemon rejected Mission remote child {ability}: {}",
                    status.message()
                ))?
                .into_inner(),
            () = self.parent.wait_for_cancel() => {
                anyhow::bail!("Mission parent cancelled while remote child {ability} was running")
            }
            () = wait_for_deadline(inherited_deadline) => {
                anyhow::bail!("Mission parent deadline elapsed while remote child {ability} was running")
            }
        };
        let trust_path = crate::daemon::trust::anchor::trust_anchor_path_from_env_or_default();
        let trust_anchor =
            crate::daemon::trust::anchor::RealmTrustAnchor::load_or_empty(&trust_path)
                .with_context(|| {
                    format!(
                        "load realm trust anchor for Mission remote child: {}",
                        trust_path.display()
                    )
                })?;
        let resolver = crate::daemon::trust::key_resolver::RealmTrustAnchorKeyResolver::new(
            crate::daemon::trust::cell::SharedTrustAnchor::new(Arc::new(trust_anchor)),
        );
        let finalized =
            crate::daemon::invocation::dispatch::forwarded_finalization::ForwardedFinalizedInvocation::verify_response(
                &binding,
                response,
                &resolver,
            )
            .map_err(|status| anyhow::anyhow!(
                "Mission remote child receipt verification failed: {}",
                status.message()
            ))?;
        if finalized.terminal_state != InvocationState::Completed {
            anyhow::bail!(
                "Mission remote child {ability} ended in {:?}",
                finalized.terminal_state
            );
        }
        let receipt_hash: [u8; 32] = finalized
            .terminal_receipt
            .self_hash
            .as_slice()
            .try_into()
            .map_err(|_| anyhow::anyhow!("verified terminal receipt hash is not 32 bytes"))?;
        Ok((
            decode_child_output(&finalized.output, ability)?,
            finalized.terminal_receipt.invocation_id,
            finalized.terminal_receipt.index,
            receipt_hash,
        ))
    }

    #[cfg(not(feature = "axon-pb"))]
    async fn forward_remote_child(
        &self,
        _ability: &str,
        _descriptor_request: axon_sdk::invocation::DescriptorBoundInvocationRequest,
        _signed_child: &axon_sdk::invocation::SignedEnvelope,
        _inherited_deadline: Option<tokio::time::Instant>,
        _trace_id: Option<&str>,
    ) -> anyhow::Result<(Value, String, u64, [u8; 32])> {
        anyhow::bail!("remote Mission child dispatch requires the axon-pb feature")
    }
}

fn decode_child_output(output: &[u8], ability: &str) -> anyhow::Result<Value> {
    if output.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(output)
        .with_context(|| format!("decode Mission child {ability} result as JSON"))
}

fn canonical_invocation_ura(
    invocation_id: &str,
    envelope: &axon_sdk::invocation::InvocationEnvelope,
) -> anyhow::Result<String> {
    axon_sdk::ura::invocation_record_ura_for_binding(
        &envelope.subject.ura,
        &envelope.callee.ura,
        &envelope.caller.ura,
        invocation_id,
    )
    .ok_or_else(|| {
        anyhow::anyhow!(
            "derive canonical Mission child Invocation URA for receipt {invocation_id:?}"
        )
    })
}

async fn wait_for_deadline(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending::<()>().await,
    }
}

fn causal_context_projection(causal: &axon_sdk::invocation::CausalContext) -> Value {
    match causal {
        axon_sdk::invocation::CausalContext::None => serde_json::json!({"kind": "none"}),
        axon_sdk::invocation::CausalContext::Scalar(receipt) => serde_json::json!({
            "kind": "scalar",
            "receipt_hash": hex::encode(receipt.receipt_hash),
            "receipt_ura": receipt.receipt_ura,
        }),
        axon_sdk::invocation::CausalContext::List(receipts) => serde_json::json!({
            "kind": "list",
            "receipts": receipts.iter().map(|receipt| serde_json::json!({
                "receipt_hash": hex::encode(receipt.receipt_hash),
                "receipt_ura": receipt.receipt_ura,
            })).collect::<Vec<_>>(),
        }),
        axon_sdk::invocation::CausalContext::Merkle { root, proof_ura } => serde_json::json!({
            "kind": "merkle",
            "root": hex::encode(root),
            "proof_ura": proof_ura,
        }),
    }
}

impl MissionInvocationGateway for DaemonMissionInvocationGateway {
    fn invoke(
        &self,
        request: MissionInvocationRequest,
    ) -> anyhow::Result<MissionInvocationOutcome> {
        crate::support::async_bridge::run_blocking(
            self.invoke_child(request),
            crate::support::async_bridge::NoRuntimeFallback::BuildCurrentThreadTokio,
        )
    }
}

#[cfg(test)]
pub(crate) struct CatalogMissionInvocationGateway {
    catalog: Arc<crate::daemon::ability::dispatch::AxonAbilityCatalog>,
}

#[cfg(test)]
impl CatalogMissionInvocationGateway {
    pub(crate) fn new(catalog: Arc<crate::daemon::ability::dispatch::AxonAbilityCatalog>) -> Self {
        Self { catalog }
    }
}

#[cfg(test)]
impl MissionInvocationGateway for CatalogMissionInvocationGateway {
    fn invoke(
        &self,
        request: MissionInvocationRequest,
    ) -> anyhow::Result<MissionInvocationOutcome> {
        let ability = match request.target {
            MissionInvocationTarget::HostedAgent(agent) => format!("{agent}.{}", request.ability),
            MissionInvocationTarget::LocalDevice | MissionInvocationTarget::RemoteNode(_) => {
                request.ability
            }
        };
        let invocation_target =
            crate::daemon::invocation::routing::target::InvocationTarget::local_daemon_system(
                ability.clone(),
                request.args,
                crate::daemon::invocation::routing::target::CallMode::Rpc,
            );
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.catalog.invoke_rpc_target_json(invocation_target)
        }));
        match result {
            Ok(result) => result.map(|value| MissionInvocationOutcome {
                value,
                invocation: MissionInvocationRecord::for_test(&ability, 0x63),
            }),
            Err(payload) => {
                let message = if let Some(value) = payload.downcast_ref::<&'static str>() {
                    (*value).to_string()
                } else if let Some(value) = payload.downcast_ref::<String>() {
                    value.clone()
                } else {
                    "non-string panic payload".to_string()
                };
                anyhow::bail!("test Invocation handler {ability} panicked: {message}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use axon_sdk::invocation::axiom::sign_descriptor_bound_invocation;
    use axon_sdk::invocation::{
        make_ability, AbilityFn, AbilityOptions, AxonError, CallerSignature, CausalContext,
        DescriptorBoundEnvelope, DescriptorBoundEnvelopeParts, DescriptorBoundInvocationRequest,
        InvocationHandle, InvocationSigningAuthority, InvocationSigningAuthorityProvider,
        KeyResolver, SignedEnvelope,
    };
    use ed25519_dalek::{SigningKey, VerifyingKey};
    use tokio::sync::mpsc;

    const DESCRIPTOR_VERSION: &str = "mission-child.v1";
    const SCHEMA_HASH: [u8; 32] = [0x31; 32];
    const IMPL_HASH: [u8; 32] = [0x42; 32];
    const DESCRIPTOR_HASH: [u8; 32] = [0x53; 32];
    const TRACE_ID: &str = "mission-parent-trace";

    #[derive(Default)]
    struct TestKeyResolver {
        keys: HashMap<String, VerifyingKey>,
    }

    impl TestKeyResolver {
        fn with_key(mut self, identity: &AgentIdentity, key: VerifyingKey) -> Self {
            self.keys.insert(identity.ura.clone(), key);
            self
        }
    }

    impl KeyResolver for TestKeyResolver {
        fn resolve(&self, agent_ura: &str) -> Result<VerifyingKey, AxonError> {
            self.keys.get(agent_ura).copied().ok_or_else(|| {
                AxonError::invalid_argument(format!("unknown Mission test identity {agent_ura}"))
            })
        }
    }

    struct FixedChildTargetResolver {
        callee_ura: String,
    }

    impl MissionChildTargetResolver for FixedChildTargetResolver {
        fn callee_ura(&self, _request: &MissionInvocationRequest) -> anyhow::Result<String> {
            Ok(self.callee_ura.clone())
        }
    }

    #[derive(Debug)]
    struct ChildObservation {
        invocation_id: String,
        signed_envelope: SignedEnvelope,
        trace_metadata: Option<String>,
        absolute_deadline: Option<tokio::time::Instant>,
    }

    fn identity(ura: &str) -> AgentIdentity {
        AgentIdentity::new(ura, UraProfile::StrictV2)
    }

    fn subject(name: &str) -> SubjectIdentity {
        SubjectIdentity::new(
            crate::core::ura::resource_dot_ura("mission-test", name, ""),
            UraProfile::StrictV2,
        )
    }

    fn runtime_ability(callee: &AgentIdentity, ability: &str) -> String {
        crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(&callee.ura, ability)
            .expect("canonical runtime ability")
    }

    fn descriptor_ref(callee: &AgentIdentity, ability: &str) -> String {
        let descriptor_binding =
            crate::daemon::axon_bridge::descriptor_ref::descriptor_binding_for_wire(
                DESCRIPTOR_VERSION,
                DESCRIPTOR_HASH,
                "invoke",
            )
            .expect("canonical descriptor binding");
        crate::daemon::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
            &callee.ura,
            ability,
            &descriptor_binding,
        )
        .expect("canonical descriptor ref")
    }

    fn proof_options() -> AbilityOptions {
        AbilityOptions::default().with_descriptor_proof(
            DESCRIPTOR_VERSION,
            "invoke",
            DESCRIPTOR_HASH,
            SCHEMA_HASH,
            IMPL_HASH,
        )
    }

    fn save_hosted_agents(
        entries: Vec<crate::daemon::persistence::local_agents::HostedAgentEntry>,
    ) {
        crate::daemon::persistence::local_agents::save(
            &crate::daemon::persistence::local_agents::LocalAgentsFile {
                host_device_agent_ura: "easynet:///r/mission-test/device/dev-1".to_string(),
                hosted_agents: entries,
            },
        )
        .expect("save hosted agents");
    }

    fn hosted_entry(
        profile: &str,
        name: &str,
        agent_ura: &str,
    ) -> crate::daemon::persistence::local_agents::HostedAgentEntry {
        crate::daemon::persistence::local_agents::HostedAgentEntry {
            profile: profile.to_string(),
            name: name.to_string(),
            agent_ura: agent_ura.to_string(),
            signing_authority: "hosted_by:easynet:///r/mission-test/device/dev-1".to_string(),
            first_seen_at: "2026-07-16T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn persisted_target_resolver_uses_aggregate_hosted_agent_identity() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        save_hosted_agents(vec![hosted_entry(
            "llm",
            "worker",
            "easynet:///r/mission-test/agent/user.worker",
        )]);
        let resolver = PersistedMissionChildTargetResolver;

        let callee = resolver
            .callee_ura(&MissionInvocationRequest::hosted_agent(
                "worker",
                "demo.run",
                serde_json::json!({}),
            ))
            .expect("resolve hosted Mission child");

        assert_eq!(callee, "easynet:///r/mission-test/agent/user.worker");
    }

    #[test]
    fn persisted_target_resolver_preserves_ambiguous_hosted_agent_error() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        save_hosted_agents(vec![
            hosted_entry("llm", "same", "easynet:///r/mission-test/agent/user.same"),
            hosted_entry(
                "mcp",
                "same",
                "easynet:///r/mission-test/agent/user.same-mcp",
            ),
        ]);
        let resolver = PersistedMissionChildTargetResolver;

        let error = resolver
            .callee_ura(&MissionInvocationRequest::hosted_agent(
                "same",
                "demo.run",
                serde_json::json!({}),
            ))
            .expect_err("ambiguous hosted Agent name must fail");

        assert!(
            format!("{error}").contains("ambiguous"),
            "error should preserve ambiguity: {error}"
        );
    }

    fn runtime_with_parent_authority(
        parent: &AgentIdentity,
        parent_key: SigningKey,
        admission_keys: TestKeyResolver,
    ) -> Arc<LocalRuntime> {
        let admission_keys = admission_keys.with_key(parent, parent_key.verifying_key());
        let provider = MissionTestInvocationSigningAuthorityProvider {
            authorities: HashMap::from([(
                parent.ura.clone(),
                Arc::new(MissionTestInvocationSigningAuthority {
                    owner: parent.clone(),
                    signing_key: parent_key,
                }) as Arc<dyn InvocationSigningAuthority>,
            )]),
        };
        LocalRuntime::new_with_authority_providers(
            Arc::new(admission_keys),
            Some(Arc::new(provider)),
            crate::daemon::axon_bridge::runtime_factory::ephemeral_test_canonical_receipt_provider(
            ),
        )
    }

    struct MissionTestInvocationSigningAuthority {
        owner: AgentIdentity,
        signing_key: SigningKey,
    }

    #[async_trait::async_trait]
    impl InvocationSigningAuthority for MissionTestInvocationSigningAuthority {
        fn owner_identity(&self) -> &AgentIdentity {
            &self.owner
        }

        async fn sign_descriptor_bound_invocation(
            &self,
            envelope: &DescriptorBoundEnvelope,
        ) -> Result<CallerSignature, AxonError> {
            if envelope.envelope().caller != self.owner {
                return Err(AxonError::permission_denied(
                    "mission_test_invocation_signer_caller_mismatch",
                ));
            }
            Ok(sign_descriptor_bound_invocation(
                &self.signing_key,
                envelope,
                "mission-parent-key",
            ))
        }
    }

    struct MissionTestInvocationSigningAuthorityProvider {
        authorities: HashMap<String, Arc<dyn InvocationSigningAuthority>>,
    }

    #[async_trait::async_trait]
    impl InvocationSigningAuthorityProvider for MissionTestInvocationSigningAuthorityProvider {
        async fn resolve(
            &self,
            caller_ura: &str,
        ) -> Result<Option<Arc<dyn InvocationSigningAuthority>>, AxonError> {
            Ok(self.authorities.get(caller_ura).cloned())
        }
    }

    async fn register_rpc(
        runtime: &Arc<LocalRuntime>,
        callee: &AgentIdentity,
        ability: &str,
        handler: AbilityFn,
    ) {
        runtime
            .register_ability_with_options(
                runtime_ability(callee, ability),
                handler,
                proof_options(),
            )
            .await
            .expect("register Mission test ability");
    }

    async fn invoke_parent(
        runtime: &Arc<LocalRuntime>,
        caller: &AgentIdentity,
        callee: &AgentIdentity,
        caller_key: &SigningKey,
        ability: &str,
        parent_subject: SubjectIdentity,
        supervisor: Option<SupervisorSpec>,
        trace_id: Option<&str>,
    ) -> InvocationHandle {
        let payload =
            serde_json::to_vec(&serde_json::json!({"run": true})).expect("encode parent payload");
        let envelope = DescriptorBoundEnvelope::from_parts(DescriptorBoundEnvelopeParts {
            caller: caller.clone(),
            callee: callee.clone(),
            ability: descriptor_ref(callee, ability),
            subject: parent_subject,
            invocation_nonce: [0xA5; 16],
            causal_context: CausalContext::None,
            args_bytes: &payload,
        })
        .expect("construct parent envelope");
        let mut request =
            DescriptorBoundInvocationRequest::signed(CallMode::Rpc, envelope, payload, caller_key)
                .with_supervisor(supervisor);
        if let Some(trace_id) = trace_id {
            request = request
                .with_trace_id(trace_id)
                .with_request_metadata(HashMap::from([(
                    AXON_TRACE_CONTEXT_METADATA_KEY.to_string(),
                    trace_id.to_string(),
                )]));
        }
        runtime
            .invoke_descriptor_bound_request_async(request)
            .await
            .expect("admit parent Invocation")
            .0
    }

    fn gateway_error(error: anyhow::Error) -> AxonError {
        AxonError::internal(format!("Mission child gateway failed: {error:#}"))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn child_is_receipt_anchored_and_inherits_subject_trace_and_parent_deadline() {
        let root_caller = identity("easynet:///r/mission-test/agent/root.caller");
        let parent_callee = identity("easynet:///r/mission-test/device/parent-device");
        let child_callee = identity("easynet:///r/mission-test/agent/worker.child");
        let root_key = SigningKey::from_bytes(&[0x11; 32]);
        let runtime = runtime_with_parent_authority(
            &parent_callee,
            SigningKey::from_bytes(&[0x22; 32]),
            TestKeyResolver::default().with_key(&root_caller, root_key.verifying_key()),
        );

        let (child_tx, mut child_rx) = mpsc::unbounded_channel();
        register_rpc(
            &runtime,
            &child_callee,
            "child.observe",
            make_ability(move |ctx: Arc<AbilityContext>| {
                let child_tx = child_tx.clone();
                async move {
                    child_tx
                        .send(ChildObservation {
                            invocation_id: ctx.invocation_id.clone(),
                            signed_envelope: ctx
                                .signed_envelope()
                                .cloned()
                                .expect("child has signed envelope"),
                            trace_metadata: ctx
                                .request_metadata
                                .get(AXON_TRACE_CONTEXT_METADATA_KEY)
                                .cloned(),
                            absolute_deadline: ctx.supervisor.absolute_deadline(),
                        })
                        .expect("record child observation");
                    serde_json::to_vec(&serde_json::json!({"observed": true}))
                        .map_err(|error| AxonError::internal(error.to_string()))
                }
            }),
        )
        .await;

        let (parent_deadline_tx, mut parent_deadline_rx) = mpsc::unbounded_channel();
        let (record_tx, mut record_rx) = mpsc::unbounded_channel();
        let runtime_for_parent = Arc::clone(&runtime);
        let child_ura = child_callee.ura.clone();
        register_rpc(
            &runtime,
            &parent_callee,
            "parent.run",
            make_ability(move |ctx: Arc<AbilityContext>| {
                let runtime = Arc::clone(&runtime_for_parent);
                let child_ura = child_ura.clone();
                let parent_deadline_tx = parent_deadline_tx.clone();
                let record_tx = record_tx.clone();
                async move {
                    parent_deadline_tx
                        .send(ctx.supervisor.absolute_deadline())
                        .expect("record parent deadline");
                    let gateway = DaemonMissionInvocationGateway::from_runtime_context(
                        Arc::clone(&ctx),
                        runtime,
                        MissionChildAdmission::CanonicalOnly,
                        Arc::new(FixedChildTargetResolver {
                            callee_ura: child_ura,
                        }),
                    )
                    .map_err(gateway_error)?;
                    let outcome = gateway
                        .invoke(
                            MissionInvocationRequest::system(
                                "child.observe",
                                serde_json::json!({"value": 17}),
                            )
                            .with_timeout(Duration::from_secs(30)),
                        )
                        .map_err(gateway_error)?;
                    record_tx
                        .send(outcome.invocation.projection())
                        .expect("record canonical child receipt");
                    serde_json::to_vec(&outcome.value)
                        .map_err(|error| AxonError::internal(error.to_string()))
                }
            }),
        )
        .await;

        let expected_subject = subject("mission-run/17");
        let parent = invoke_parent(
            &runtime,
            &root_caller,
            &parent_callee,
            &root_key,
            "parent.run",
            expected_subject.clone(),
            Some(SupervisorSpec {
                resource_limit: ResourceLimit {
                    wall_seconds: Some(10.0),
                    ..Default::default()
                },
                ..Default::default()
            }),
            Some(TRACE_ID),
        )
        .await;
        let child = tokio::time::timeout(Duration::from_secs(2), child_rx.recv())
            .await
            .expect("child observation timeout")
            .expect("child observation channel closed");
        let parent_deadline = parent_deadline_rx
            .recv()
            .await
            .expect("parent deadline observation");
        let invocation_record = record_rx
            .recv()
            .await
            .expect("canonical child receipt observation");
        assert_eq!(parent.wait().await, InvocationState::Completed);

        let parent_receipt = parent
            .admission_receipt()
            .await
            .expect("parent admission receipt");
        let child_anchor = match &child.signed_envelope.envelope.causal_context {
            CausalContext::Scalar(anchor) => anchor,
            other => panic!("Mission child must carry one parent receipt anchor, got {other:?}"),
        };
        assert_eq!(child_anchor.receipt_hash, parent_receipt.self_hash());
        assert!(child_anchor.receipt_ura.contains(parent.invocation_id()));
        assert_eq!(child.signed_envelope.envelope.caller, parent_callee);
        assert_eq!(child.signed_envelope.envelope.callee, child_callee);
        assert_eq!(child.signed_envelope.envelope.subject, expected_subject);
        assert!(child
            .signed_envelope
            .envelope
            .ability
            .contains("child.observe"));
        assert_ne!(child.signed_envelope.envelope.invocation_nonce, [0_u8; 16]);
        assert_ne!(child.signed_envelope.envelope.args_digest, [0_u8; 32]);
        assert_eq!(child.trace_metadata.as_deref(), Some(TRACE_ID));
        assert_eq!(child.absolute_deadline, parent_deadline);
        let child_invocation_ura = invocation_record["invocation_ura"]
            .as_str()
            .expect("child record has canonical Invocation URA");
        assert!(child_invocation_ura.starts_with("easynet:///"));
        assert!(invocation_record["receipt"]["anchor"]["receipt_ura"]
            .as_str()
            .is_some_and(|receipt_ura| receipt_ura.starts_with(child_invocation_ura)));
        assert!(invocation_record["receipt"]["anchor"]["receipt_hash"]
            .as_str()
            .is_some_and(|receipt_hash| receipt_hash.len() == 64));

        let child_core = runtime
            .invocation_snapshot(&child.invocation_id)
            .await
            .expect("child invocation snapshot");
        assert_eq!(
            child_core.parent_invocation_id.as_deref(),
            Some(parent.invocation_id())
        );
        assert_eq!(
            runtime.children_of(parent.invocation_id()).await,
            vec![child.invocation_id]
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn cancelling_parent_cancels_gateway_child() {
        let root_caller = identity("easynet:///r/mission-test/agent/cancel.root");
        let parent_callee = identity("easynet:///r/mission-test/device/cancel-parent");
        let child_callee = identity("easynet:///r/mission-test/agent/cancel.child");
        let root_key = SigningKey::from_bytes(&[0x33; 32]);
        let runtime = runtime_with_parent_authority(
            &parent_callee,
            SigningKey::from_bytes(&[0x44; 32]),
            TestKeyResolver::default().with_key(&root_caller, root_key.verifying_key()),
        );

        let (child_tx, mut child_rx) = mpsc::unbounded_channel();
        register_rpc(
            &runtime,
            &child_callee,
            "child.wait",
            make_ability(move |ctx: Arc<AbilityContext>| {
                let child_tx = child_tx.clone();
                async move {
                    child_tx
                        .send(ctx.invocation_id.clone())
                        .expect("record child id");
                    ctx.wait_for_cancel().await;
                    Ok(Vec::new())
                }
            }),
        )
        .await;

        let runtime_for_parent = Arc::clone(&runtime);
        let child_ura = child_callee.ura.clone();
        register_rpc(
            &runtime,
            &parent_callee,
            "parent.wait",
            make_ability(move |ctx: Arc<AbilityContext>| {
                let runtime = Arc::clone(&runtime_for_parent);
                let child_ura = child_ura.clone();
                async move {
                    let gateway = DaemonMissionInvocationGateway::from_runtime_context(
                        Arc::clone(&ctx),
                        runtime,
                        MissionChildAdmission::CanonicalOnly,
                        Arc::new(FixedChildTargetResolver {
                            callee_ura: child_ura,
                        }),
                    )
                    .map_err(gateway_error)?;
                    gateway
                        .invoke(MissionInvocationRequest::system(
                            "child.wait",
                            serde_json::json!({}),
                        ))
                        .map(|outcome| outcome.value)
                        .map_err(gateway_error)?;
                    Ok(Vec::new())
                }
            }),
        )
        .await;

        let parent = invoke_parent(
            &runtime,
            &root_caller,
            &parent_callee,
            &root_key,
            "parent.wait",
            subject("cancel-run"),
            None,
            None,
        )
        .await;
        let child_id = tokio::time::timeout(Duration::from_secs(2), child_rx.recv())
            .await
            .expect("child launch timeout")
            .expect("child id channel closed");
        parent
            .cancel("mission_parent_cancelled")
            .await
            .expect("cancel parent");

        assert_eq!(parent.wait().await, InvocationState::Cancelled);
        let child_state = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let state = runtime
                    .invocation_snapshot(&child_id)
                    .await
                    .expect("child invocation snapshot")
                    .state;
                if state.is_terminal() {
                    break state;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("child cancellation timeout");
        assert_eq!(child_state, InvocationState::Cancelled);
    }
}
