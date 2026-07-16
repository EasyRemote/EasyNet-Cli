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
use easynet_axon::invocation::{
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
    hosted_agent: Option<String>,
    timeout: Duration,
}

impl MissionInvocationRequest {
    /// Invoke a device-owned system ability.
    pub(crate) fn system(ability: impl Into<String>, args: Value) -> Self {
        Self {
            ability: ability.into(),
            args,
            hosted_agent: None,
            timeout: default_child_timeout(),
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
            hosted_agent: Some(agent.into()),
            timeout: default_child_timeout(),
        }
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
        self.hosted_agent.as_deref()
    }
}

fn default_child_timeout() -> Duration {
    Duration::from_secs(crate::support::platform::timeouts::INVOKE_DEFAULT_SECS)
}

/// Mission-facing port for daemon-owned canonical Invocation.
pub(crate) trait MissionInvocationGateway: Send + Sync {
    fn invoke(&self, request: MissionInvocationRequest) -> anyhow::Result<Value>;
}

trait MissionChildTargetResolver: Send + Sync {
    fn callee_ura(&self, request: &MissionInvocationRequest) -> anyhow::Result<String>;
}

#[derive(Debug, Default)]
struct PersistedMissionChildTargetResolver;

impl MissionChildTargetResolver for PersistedMissionChildTargetResolver {
    fn callee_ura(&self, request: &MissionInvocationRequest) -> anyhow::Result<String> {
        let Some(agent_name) = request.hosted_agent.as_deref() else {
            return Ok(crate::daemon::identity::local_invocation::local_device_ura());
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
    parent_subject: SubjectIdentity,
    trace_id: Option<String>,
    target_resolver: Arc<dyn MissionChildTargetResolver>,
}

impl DaemonMissionInvocationGateway {
    pub(crate) fn from_envelope(
        envelope: &EnvelopeContext,
        runtime: Arc<LocalRuntime>,
    ) -> anyhow::Result<Self> {
        let parent = envelope.runtime_invocation_context().ok_or_else(|| {
            anyhow::anyhow!("Mission child dispatch requires an admitted Axon parent capability")
        })?;
        Self::from_runtime_context(
            parent,
            runtime,
            Arc::new(PersistedMissionChildTargetResolver),
        )
    }

    fn from_runtime_context(
        parent: Arc<AbilityContext>,
        runtime: Arc<LocalRuntime>,
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
            parent_subject,
            trace_id,
            target_resolver,
        })
    }

    async fn invoke_child(&self, request: MissionInvocationRequest) -> anyhow::Result<Value> {
        let MissionInvocationRequest {
            ability,
            args,
            hosted_agent: _,
            timeout,
        } = request.clone();
        let callee_ura = self.target_resolver.callee_ura(&request)?;
        let runtime_ability =
            crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(&callee_ura, &ability)
                .map_err(|error| anyhow::anyhow!("resolve Mission child ability URA: {error}"))?;
        let descriptor_binding =
            crate::daemon::axon_bridge::descriptor_ref::registered_descriptor_binding(
                &self.runtime,
                &runtime_ability,
                CallMode::Rpc,
            )
            .await
            .map_err(|error| anyhow::anyhow!("resolve Mission child descriptor: {error}"))?;
        let descriptor_ref =
            crate::daemon::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
                &callee_ura,
                &ability,
                &descriptor_binding,
            )
            .map_err(|error| anyhow::anyhow!("bind Mission child descriptor: {error}"))?;
        let target = DescriptorBoundInvocationTarget::new(
            AgentIdentity::new(callee_ura, UraProfile::EasynetStrictV2),
            descriptor_ref,
        )
        .map_err(|error| anyhow::anyhow!("construct Mission child target: {error}"))?;
        let payload = serde_json::to_vec(&args).context("encode Mission child arguments")?;
        let child = ChildInvocationRequest::new(target, self.parent_subject.clone(), payload)
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
        let mut descriptor_request = prepared.into_descriptor_request();
        if let Some(trace_id) = self.trace_id.as_deref() {
            descriptor_request = descriptor_request
                .with_trace_id(trace_id)
                .with_request_metadata(HashMap::from([(
                    AXON_TRACE_CONTEXT_METADATA_KEY.to_string(),
                    trace_id.to_string(),
                )]));
        }
        let (handle, _signed_child) = self
            .runtime
            .invoke_descriptor_bound_request_async(descriptor_request)
            .await
            .map_err(|error| anyhow::anyhow!("admit Mission child {ability}: {error}"))?;
        let finalized = handle
            .finalized()
            .await
            .map_err(|error| anyhow::anyhow!("finalize Mission child {ability}: {error}"))?;
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
        if finalized.output().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_slice(finalized.output())
            .with_context(|| format!("decode Mission child {ability} result as JSON"))
    }
}

impl MissionInvocationGateway for DaemonMissionInvocationGateway {
    fn invoke(&self, request: MissionInvocationRequest) -> anyhow::Result<Value> {
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
    fn invoke(&self, request: MissionInvocationRequest) -> anyhow::Result<Value> {
        let ability = match request.hosted_agent {
            Some(agent) => format!("{agent}.{}", request.ability),
            None => request.ability,
        };
        let invocation_target = crate::daemon::invocation::routing::target::InvocationTarget {
            scope: crate::daemon::invocation::routing::target::TargetScope::Local,
            ability: ability.clone(),
            normalized_args: request.args,
            call_mode: crate::daemon::invocation::routing::target::CallMode::Rpc,
            subject: crate::daemon::invocation::routing::target::InvocationSubject::daemon_system_derived(),
            causal_context:
                crate::daemon::invocation::routing::target::InvocationCausalContext::daemon_system_root(),
            request_metadata: HashMap::new(),
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.catalog.invoke_rpc_target_json(invocation_target)
        }));
        match result {
            Ok(result) => result,
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

    use easynet_axon::invocation::{
        make_ability, AbilityFn, AbilityOptions, AxonError, CausalContext, DescriptorBoundEnvelope,
        DescriptorBoundEnvelopeParts, DescriptorBoundInvocationRequest,
        Ed25519InvocationSigningAuthority, InvocationHandle, InvocationSigningAuthority,
        KeyResolver, SignedEnvelope, StaticInvocationSigningAuthorityProvider,
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
        AgentIdentity::new(ura, UraProfile::EasynetStrictV2)
    }

    fn subject(name: &str) -> SubjectIdentity {
        SubjectIdentity::new(
            crate::core::ura::resource_dot_ura("mission-test", name, ""),
            UraProfile::EasynetStrictV2,
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
    ) -> Arc<LocalRuntime> {
        let authority: Arc<dyn InvocationSigningAuthority> =
            Arc::new(Ed25519InvocationSigningAuthority::self_signed(
                parent.clone(),
                parent_key,
                "mission-parent-key",
            ));
        let mut provider = StaticInvocationSigningAuthorityProvider::new();
        provider.insert(authority).expect("insert parent authority");
        LocalRuntime::new_with_signing_authority_providers(
            Some(Arc::new(provider)),
            crate::daemon::axon_bridge::runtime_factory::ephemeral_test_receipt_signing_authority_provider(),
        )
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
        let parent_key = SigningKey::from_bytes(&[0x22; 32]);
        let runtime =
            runtime_with_parent_authority(&parent_callee, SigningKey::from_bytes(&[0x22; 32]));
        runtime.set_admission_key_resolver(Arc::new(
            TestKeyResolver::default()
                .with_key(&root_caller, root_key.verifying_key())
                .with_key(&parent_callee, parent_key.verifying_key()),
        ));

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
                async move {
                    parent_deadline_tx
                        .send(ctx.supervisor.absolute_deadline())
                        .expect("record parent deadline");
                    let gateway = DaemonMissionInvocationGateway::from_runtime_context(
                        Arc::clone(&ctx),
                        runtime,
                        Arc::new(FixedChildTargetResolver {
                            callee_ura: child_ura,
                        }),
                    )
                    .map_err(gateway_error)?;
                    let result = gateway
                        .invoke(
                            MissionInvocationRequest::system(
                                "child.observe",
                                serde_json::json!({"value": 17}),
                            )
                            .with_timeout(Duration::from_secs(30)),
                        )
                        .map_err(gateway_error)?;
                    serde_json::to_vec(&result)
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
        assert_eq!(child.trace_metadata.as_deref(), Some(TRACE_ID));
        assert_eq!(child.absolute_deadline, parent_deadline);

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
        let parent_key = SigningKey::from_bytes(&[0x44; 32]);
        let runtime =
            runtime_with_parent_authority(&parent_callee, SigningKey::from_bytes(&[0x44; 32]));
        runtime.set_admission_key_resolver(Arc::new(
            TestKeyResolver::default()
                .with_key(&root_caller, root_key.verifying_key())
                .with_key(&parent_callee, parent_key.verifying_key()),
        ));

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
