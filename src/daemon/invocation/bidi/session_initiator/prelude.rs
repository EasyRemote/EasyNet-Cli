use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axon_sdk::pb::axon::v1::{invocation_client::InvocationClient, InvokeRequest};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use tonic::{transport::Channel, Status};

use super::heartbeat::spawn_federation_heartbeat;
use super::supervisor::{DeviceSessionPhase, PreludeStep, SessionPhaseTracker};
use super::tasks::AbortOnDrop;
use super::SessionError;
use crate::daemon::ability::descriptors::AbilityDescriptor;
use crate::daemon::federation::read_model::authority_published_abilities::AuthorityPublishedAbilityStore;
use crate::daemon::identity::self_identity::{CanonicalSigner, SelfIdentityError};
use crate::daemon::invocation::admission::register_device_pubkey::RegisterPubkeyRequest;
use crate::daemon::persistence::agent_aggregate::{
    AgentAggregateRepository, AgentHostedAdvertiseEntry,
};
use crate::daemon::trust::anchor::TrustAnchorRole;

pub struct SessionPreludeInputs<'a> {
    pub(super) ability_descriptors: &'a [AbilityDescriptor],
    pub(super) authority_published_abilities: Arc<AuthorityPublishedAbilityStore>,
    paired_user_signer: Option<PairedUserTrustSigner>,
}

impl<'a> SessionPreludeInputs<'a> {
    #[must_use]
    pub fn new(
        ability_descriptors: &'a [AbilityDescriptor],
        authority_published_abilities: Arc<AuthorityPublishedAbilityStore>,
    ) -> Self {
        Self {
            ability_descriptors,
            authority_published_abilities,
            paired_user_signer: None,
        }
    }

    /// Supply only the paired User signing authority needed by owner projection.
    ///
    /// This is intentionally independent from [`UserTrustSync`]: publishing a
    /// User-owned Agent and continuously synchronizing trust are separate
    /// lifecycle responsibilities, even though production uses the same key
    /// source for both.
    #[must_use]
    pub fn with_paired_user_signer(mut self, signer: PairedUserTrustSigner) -> Self {
        self.paired_user_signer = Some(signer);
        self
    }
}

pub(super) struct SessionPreludeChannels {
    user_trust_resync: Channel,
    federation_heartbeat: Channel,
}

impl SessionPreludeChannels {
    pub(super) fn new(user_trust_resync: Channel, federation_heartbeat: Channel) -> Self {
        Self {
            user_trust_resync,
            federation_heartbeat,
        }
    }
}

pub(super) struct SessionPreludeRun<'a> {
    pub(super) client: &'a mut InvocationClient<Channel>,
    pub(super) phase: &'a mut SessionPhaseTracker,
    pub(super) hub_endpoint: &'a str,
    pub(super) signer: Arc<dyn CanonicalSigner>,
    pub(super) inputs: SessionPreludeInputs<'a>,
    pub(super) user_trust_sync: Option<&'a UserTrustSync>,
    pub(super) channels: SessionPreludeChannels,
}

pub(super) struct SessionPreludeGuards {
    _user_trust_resync: Option<AbortOnDrop>,
    _federation_heartbeat: AbortOnDrop,
}

pub(super) async fn run_session_preludes(
    request: SessionPreludeRun<'_>,
) -> Result<SessionPreludeGuards, SessionError> {
    let SessionPreludeRun {
        client,
        phase,
        hub_endpoint,
        signer,
        inputs,
        user_trust_sync,
        channels,
    } = request;
    let caller_ura = signer.owner_ura().to_string();
    let ability_descriptors = inputs.ability_descriptors;
    let paired_user_signer = inputs
        .paired_user_signer
        .as_ref()
        .or_else(|| user_trust_sync.map(|sync| &sync.user_signer));
    let owner_projections =
        committed_device_native_owner_descriptors(ability_descriptors, &caller_ura);
    let authority_published_abilities = inputs.authority_published_abilities;

    run_join_prelude(
        client,
        phase,
        hub_endpoint,
        &caller_ura,
        signer.as_ref(),
        &authority_published_abilities,
    )
    .await;
    run_owner_projection_prelude(
        client,
        phase,
        hub_endpoint,
        &caller_ura,
        signer.as_ref(),
        &owner_projections,
    )
    .await?;

    let user_trust_resync = run_user_trust_bootstrap_and_spawn_resync(
        client,
        phase,
        channels.user_trust_resync,
        Arc::clone(&signer),
        user_trust_sync,
    )
    .await
    .map_err(|source| SessionError::UserTrustBootstrapFailed {
        endpoint: hub_endpoint.to_string(),
        source,
    })?;
    run_user_service_owner_projection_prelude(
        client,
        phase,
        hub_endpoint,
        &caller_ura,
        signer.as_ref(),
        paired_user_signer,
        ability_descriptors,
    )
    .await?;
    let federation_heartbeat = spawn_federation_heartbeat(
        channels.federation_heartbeat,
        Arc::clone(&signer),
        Arc::clone(&authority_published_abilities),
    );

    run_hosted_agent_advertise_prelude(
        client,
        phase,
        hub_endpoint,
        signer.as_ref(),
        paired_user_signer,
        ability_descriptors,
    )
    .await?;

    Ok(SessionPreludeGuards {
        _user_trust_resync: user_trust_resync,
        _federation_heartbeat: federation_heartbeat,
    })
}

async fn run_join_prelude(
    client: &mut InvocationClient<Channel>,
    phase: &mut SessionPhaseTracker,
    hub_endpoint: &str,
    caller_ura: &str,
    signer: &dyn CanonicalSigner,
    authority_published_abilities: &AuthorityPublishedAbilityStore,
) {
    phase.transition(
        DeviceSessionPhase::Preluding(PreludeStep::Join),
        "channel_connected",
    );
    crate::op_event!(
        component = session,
        kind = federation_join_prelude_sending,
        caller_ura = caller_ura,
        hub_endpoint = hub_endpoint,
    );
    match send_federation_join_prelude(client, signer, authority_published_abilities).await {
        Ok(()) => {
            crate::op_event!(
                component = session,
                kind = federation_join_prelude_ok,
                message = "proceeding to session.open",
            );
        }
        Err(err) => {
            let code = err.code();
            let msg = err.message();
            crate::op_event!(
                component = session,
                kind = federation_join_prelude_soft_failed,
                code = code,
                error = msg,
                message =
                    "proceeding to session.open — bidi will surface the error if join was required",
            );
        }
    }
}

async fn run_owner_projection_prelude(
    client: &mut InvocationClient<Channel>,
    phase: &mut SessionPhaseTracker,
    hub_endpoint: &str,
    caller_ura: &str,
    signer: &dyn CanonicalSigner,
    owner_projections: &BTreeMap<String, Vec<AbilityDescriptor>>,
) -> Result<(), SessionError> {
    phase.transition(
        DeviceSessionPhase::Preluding(PreludeStep::OwnerProjection),
        "join_prelude_done",
    );
    if owner_projections.is_empty() {
        return Ok(());
    }

    let ability_count = owner_projections.values().map(Vec::len).sum::<usize>();
    let owner_count = owner_projections.len();
    crate::op_event!(
        component = session,
        kind = advertise_abilities_prelude_sending,
        ability_count = ability_count,
        owner_count = owner_count,
    );
    for (owner_ura, descriptors) in owner_projections {
        if let Err(status) =
            send_advertise_abilities_prelude(client, owner_ura, caller_ura, signer, descriptors)
                .await
        {
            let code = status.code();
            let msg = status.message();
            crate::op_event!(
                component = session,
                kind = advertise_abilities_prelude_failed,
                owner_ura = owner_ura,
                code = code,
                error = msg,
                message = "device-native owner projection publish failed; reconnecting instead of exposing an online host with an incomplete namespace",
            );
            return Err(SessionError::OwnerProjectionFailed {
                endpoint: hub_endpoint.to_string(),
                status,
            });
        }
    }

    crate::op_event!(
        component = session,
        kind = advertise_abilities_prelude_ok,
        ability_count = ability_count,
        owner_count = owner_count,
    );
    Ok(())
}

async fn run_user_service_owner_projection_prelude(
    client: &mut InvocationClient<Channel>,
    phase: &mut SessionPhaseTracker,
    hub_endpoint: &str,
    caller_ura: &str,
    device_signer: &dyn CanonicalSigner,
    paired_user_signer: Option<&PairedUserTrustSigner>,
    ability_descriptors: &[AbilityDescriptor],
) -> Result<(), SessionError> {
    if !ability_descriptors
        .iter()
        .any(|descriptor| is_service_owner_ura(&descriptor.owner_ura))
    {
        return Ok(());
    }
    let user_ura =
        resolve_runtime_user_ura_for_owner_projection(hub_endpoint, "user-scoped Service")?;
    let service_owner_projections =
        committed_user_service_owner_descriptors(ability_descriptors, &user_ura);
    if service_owner_projections.is_empty() {
        return Ok(());
    }
    phase.transition(
        DeviceSessionPhase::Preluding(PreludeStep::OwnerProjection),
        "user_trust_bootstrap_done",
    );
    let Some(user_signer_source) = paired_user_signer else {
        let status = tonic::Status::failed_precondition(
            "user-scoped Service owner projection requires the paired User signer",
        );
        return Err(SessionError::OwnerProjectionFailed {
            endpoint: hub_endpoint.to_string(),
            status,
        });
    };
    let user_signer = user_signer_source
        .load(&user_ura)
        .await
        .map_err(|error| SessionError::OwnerProjectionFailed {
            endpoint: hub_endpoint.to_string(),
            status: tonic::Status::failed_precondition(format!(
                "load paired User signer for user-scoped Service owner projection `{user_ura}`: {error}"
            )),
        })?;

    let ability_count = service_owner_projections
        .values()
        .map(Vec::len)
        .sum::<usize>();
    let owner_count = service_owner_projections.len();
    crate::op_event!(
        component = session,
        kind = advertise_service_abilities_prelude_sending,
        ability_count = ability_count,
        owner_count = owner_count,
    );
    for (owner_ura, descriptors) in service_owner_projections {
        match send_user_service_advertise_abilities_prelude(
            client,
            &owner_ura,
            caller_ura,
            device_signer,
            user_signer.as_ref(),
            &descriptors,
        )
        .await
        {
            Ok(UserServiceAdvertiseAbilitiesPreludeOutcome::Published) => {}
            Ok(UserServiceAdvertiseAbilitiesPreludeOutcome::ReadModelRejected {
                accepted_count,
                expected_count,
                outcome,
            }) => {
                crate::op_event!(
                    component = session,
                    kind = advertise_service_abilities_prelude_degraded,
                    owner_ura = owner_ura,
                    accepted_count = accepted_count,
                    expected_count = expected_count,
                    outcome = outcome.as_deref().unwrap_or("unknown"),
                    message = "user-scoped Service owner projection was not selected by the Hub read model; keeping the Device session online and leaving that Service surface on the existing projection",
                );
            }
            Err(status) => {
                let code = status.code();
                let msg = status.message();
                crate::op_event!(
                    component = session,
                    kind = advertise_service_abilities_prelude_failed,
                    owner_ura = owner_ura,
                    code = code,
                    error = msg,
                    message = "user-scoped Service owner projection publish failed before read-model selection; reconnecting instead of exposing an unauthorised or malformed namespace",
                );
                return Err(SessionError::OwnerProjectionFailed {
                    endpoint: hub_endpoint.to_string(),
                    status,
                });
            }
        }
    }
    crate::op_event!(
        component = session,
        kind = advertise_service_abilities_prelude_ok,
        ability_count = ability_count,
        owner_count = owner_count,
    );
    Ok(())
}

async fn run_user_trust_bootstrap_and_spawn_resync(
    client: &mut InvocationClient<Channel>,
    phase: &mut SessionPhaseTracker,
    resync_channel: Channel,
    signer: Arc<dyn CanonicalSigner>,
    user_trust_sync: Option<&UserTrustSync>,
) -> Result<Option<AbortOnDrop>, UserTrustBootstrapError> {
    let Some(sync) = user_trust_sync else {
        return Ok(None);
    };
    phase.transition(
        DeviceSessionPhase::Preluding(PreludeStep::TrustBootstrap),
        "owner_projection_published",
    );
    sync_realm_hub_trust_prelude(client, signer.as_ref(), sync).await;
    let outcome = sync_paired_user_trust_prelude(client, sync).await?;
    log_user_trust_bootstrap_outcome(&outcome);
    let sync = sync.clone();
    Ok(Some(AbortOnDrop(tokio::spawn(async move {
        let mut resync_client =
            crate::daemon::invocation::transport::invocation_client(resync_channel);
        loop {
            tokio::time::sleep(USER_TRUST_RESYNC_INTERVAL).await;
            sync_realm_hub_trust_prelude(&mut resync_client, signer.as_ref(), &sync).await;
            if let Err(err) = sync_paired_user_trust_prelude(&mut resync_client, &sync).await {
                let error = err.to_string();
                crate::op_event!(
                    component = session,
                    kind = user_trust_resync_failed,
                    error = error,
                );
            }
        }
    }))))
}

async fn run_hosted_agent_advertise_prelude(
    client: &mut InvocationClient<Channel>,
    phase: &mut SessionPhaseTracker,
    hub_endpoint: &str,
    signer: &dyn CanonicalSigner,
    paired_user_signer: Option<&PairedUserTrustSigner>,
    ability_descriptors: &[AbilityDescriptor],
) -> Result<(), SessionError> {
    let caller_ura = signer.owner_ura();
    let realm = crate::core::ura::parse_ura(caller_ura)
        .map(|parsed| parsed.realm)
        .map_err(|error| SessionError::HostedAgentPreludeFailed {
            endpoint: hub_endpoint.to_string(),
            reason: format!("signer owner URA `{caller_ura}` is invalid: {error}"),
        })?;
    if crate::daemon::persistence::config::load_credentials_optional()
        .map_err(|error| SessionError::HostedAgentPreludeFailed {
            endpoint: hub_endpoint.to_string(),
            reason: format!("load credentials for hosted-agent owner projection: {error}"),
        })?
        .as_ref()
        .is_some_and(|credentials| {
            matches!(
                credentials.runtime_user_binding(),
                Ok(crate::daemon::persistence::config::RuntimeUserBinding::Unbound { .. })
            )
        })
    {
        crate::op_event!(
            component = session,
            kind = advertise_agent_prelude_skipped,
            reason = "runtime_user_unbound",
            message = "device-only runtime has no user-root hosted-agent owner projection",
        );
        return Ok(());
    }
    let user_segment = resolve_hosted_agent_user_segment(hub_endpoint)?;
    let user_ura = crate::core::ura::user_ura(&realm, &user_segment);
    let user_signer = paired_user_signer
        .ok_or_else(|| SessionError::HostedAgentPreludeFailed {
            endpoint: hub_endpoint.to_string(),
            reason: format!(
                "hosted-agent owner projection for `{user_ura}` requires the paired User signer source"
            ),
        })?
        .load(&user_ura)
        .await
        .map_err(|error| SessionError::HostedAgentPreludeFailed {
            endpoint: hub_endpoint.to_string(),
            reason: format!(
                "load paired User signer for hosted-agent owner projection `{user_ura}`: {error}"
            ),
        })?;

    let hosted_identity =
        AgentAggregateRepository::load_hosted_identity_snapshot().map_err(|error| {
            SessionError::HostedAgentPreludeFailed {
                endpoint: hub_endpoint.to_string(),
                reason: format!("load hosted Agent identity projection: {error}"),
            }
        })?;
    let entries = hosted_identity.hosted_advertise_entries(&realm, &user_segment);
    if realm.is_empty() || entries.is_empty() {
        return Ok(());
    }

    let caller_node_id = crate::core::ura::parse_ura(caller_ura)
        .ok()
        .filter(|p| p.kind == crate::core::ura::URAKind::Device)
        .and_then(|p| p.device_id().map(str::to_string));
    let entries_count = entries.len();
    let labels_display = format!(
        "{:?}",
        entries
            .iter()
            .map(AgentHostedAdvertiseEntry::short_label)
            .collect::<Vec<_>>()
    );
    phase.transition(
        DeviceSessionPhase::Preluding(PreludeStep::Advertise),
        "projection_published",
    );
    crate::op_event!(
        component = session,
        kind = advertise_agent_prelude_sending,
        agent_count = entries_count,
        user = user_segment,
        labels = labels_display,
    );

    for entry in &entries {
        advertise_hosted_agent_entry(
            client,
            caller_ura,
            &caller_node_id,
            ability_descriptors,
            entry,
            signer,
            user_signer.as_ref(),
        )
        .await
        .map_err(|reason| SessionError::HostedAgentPreludeFailed {
            endpoint: hub_endpoint.to_string(),
            reason,
        })?;
    }
    let entries_done_count = entries.len();
    crate::op_event!(
        component = session,
        kind = advertise_agent_prelude_done,
        agent_count = entries_done_count,
    );
    Ok(())
}

fn resolve_hosted_agent_user_segment(hub_endpoint: &str) -> Result<String, SessionError> {
    let user_ura = resolve_runtime_user_ura_for_owner_projection(hub_endpoint, "hosted-agent")?;
    let parsed = crate::core::ura::parse_ura(&user_ura).map_err(|error| {
        SessionError::HostedAgentPreludeFailed {
            endpoint: hub_endpoint.to_string(),
            reason: format!(
                "project runtime user binding for hosted-agent owner projection: invalid user URA `{user_ura}`: {error}"
            ),
        }
    })?;
    parsed
        .user_id()
        .map(str::to_string)
        .ok_or_else(|| SessionError::HostedAgentPreludeFailed {
            endpoint: hub_endpoint.to_string(),
            reason: format!(
                "project runtime user binding for hosted-agent owner projection: user URA `{user_ura}` has no user id"
            ),
        })
}

fn resolve_runtime_user_ura_for_owner_projection(
    hub_endpoint: &str,
    owner_kind: &str,
) -> Result<String, SessionError> {
    let Some(credentials) = crate::daemon::persistence::config::load_credentials_optional()
        .map_err(|error| SessionError::HostedAgentPreludeFailed {
            endpoint: hub_endpoint.to_string(),
            reason: format!("load credentials for {owner_kind} owner projection: {error}"),
        })?
    else {
        return Err(SessionError::HostedAgentPreludeFailed {
            endpoint: hub_endpoint.to_string(),
            reason: format!(
                "project runtime user binding for {owner_kind} owner projection: no paired credentials are available"
            ),
        });
    };
    let user_ura = match credentials.runtime_user_binding().map_err(|error| {
        SessionError::HostedAgentPreludeFailed {
            endpoint: hub_endpoint.to_string(),
            reason: format!(
                "project runtime user binding for {owner_kind} owner projection: {error}"
            ),
        }
    })? {
        crate::daemon::persistence::config::RuntimeUserBinding::Bound { user_ura } => user_ura,
        crate::daemon::persistence::config::RuntimeUserBinding::Unbound { reason } => {
            return Err(SessionError::HostedAgentPreludeFailed {
                endpoint: hub_endpoint.to_string(),
                reason: format!(
                    "project runtime user binding for {owner_kind} owner projection: {reason}"
                ),
            });
        }
    };
    let parsed = crate::core::ura::parse_ura(&user_ura).map_err(|error| {
        SessionError::HostedAgentPreludeFailed {
            endpoint: hub_endpoint.to_string(),
            reason: format!(
                "project runtime user binding for {owner_kind} owner projection: invalid user URA `{user_ura}`: {error}"
            ),
        }
    })?;
    if parsed.kind != crate::core::ura::URAKind::User {
        return Err(SessionError::HostedAgentPreludeFailed {
            endpoint: hub_endpoint.to_string(),
            reason: format!(
                "project runtime user binding for {owner_kind} owner projection: expected User URA, got `{user_ura}`"
            ),
        });
    }
    Ok(user_ura)
}

async fn advertise_hosted_agent_entry(
    client: &mut InvocationClient<Channel>,
    caller_ura: &str,
    caller_node_id: &Option<String>,
    ability_descriptors: &[AbilityDescriptor],
    entry: &AgentHostedAdvertiseEntry,
    signer: &dyn CanonicalSigner,
    user_signer: &dyn CanonicalSigner,
) -> Result<(), String> {
    let host_for_advertise = caller_node_id.as_deref();
    let plan =
        crate::daemon::federation::hosted_agent_publication::HostedAgentPublicationPlan::begin(
            entry.agent_ura(),
            caller_ura,
            host_for_advertise,
            ability_descriptors,
        )?;
    let assignment = send_advertise_agent_prelude(
        client,
        entry.agent_ura(),
        plan.identity_payload_bytes()?,
        signer,
    )
    .await
    .map_err(|error| {
        format!(
            "advertise hosted agent `{}` failed (code={:?}): {}",
            entry.agent_ura(),
            error.code(),
            error.message()
        )
    })?;
    let active = plan.activate(assignment)?;
    let mut advertise_ctx = HostedAgentAbilityAdvertiseContext {
        client,
        device_signer: signer,
        user_signer,
    };
    advertise_hosted_agent_abilities(&mut advertise_ctx, entry, &active).await
}

struct HostedAgentAbilityAdvertiseContext<'a> {
    client: &'a mut InvocationClient<Channel>,
    device_signer: &'a dyn CanonicalSigner,
    user_signer: &'a dyn CanonicalSigner,
}

async fn advertise_hosted_agent_abilities(
    ctx: &mut HostedAgentAbilityAdvertiseContext<'_>,
    entry: &AgentHostedAdvertiseEntry,
    plan: &crate::daemon::federation::hosted_agent_publication::AssignedHostedAgentPublication,
) -> Result<(), String> {
    let ability_count = plan.ability_count;
    crate::op_event!(
        component = session,
        kind = advertise_hosted_agent_abilities_prelude_sending,
        agent_ura = entry.agent_ura(),
        ability_count = ability_count,
    );
    send_prepared_advertise_abilities_prelude(
        ctx.client,
        entry.agent_ura(),
        ctx.device_signer,
        PreludeOwnerProjectionAuthority::UserDelegation(ctx.user_signer),
        &plan.publication,
    )
    .await
    .map_err(|error| {
        format!(
            "advertise hosted-agent abilities for `{}` failed (code={:?}): {}",
            entry.agent_ura(),
            error.code(),
            error.message()
        )
    })?;
    plan.mark_published()?;
    crate::op_event!(
        component = session,
        kind = advertise_hosted_agent_abilities_prelude_ok,
        agent_ura = entry.agent_ura(),
        ability_count = ability_count,
    );
    Ok(())
}

async fn send_federation_join_prelude(
    client: &mut InvocationClient<Channel>,
    signer: &dyn CanonicalSigner,
    authority_published_abilities: &AuthorityPublishedAbilityStore,
) -> Result<(), tonic::Status> {
    let caller_ura = signer.owner_ura();
    let realm = crate::core::ura::parse_ura(caller_ura)
        .map(|parsed| parsed.realm)
        .unwrap_or_default();

    let body = crate::daemon::federation::client::ability_contract::JoinArgs {
        realm,
        membership_ura: caller_ura.to_string(),
        public_key_hex: federation_join_public_key_hex(signer)?,
        principal_enrollment: None,
    };
    let arguments = serde_json::to_vec(&body)
        .map_err(|e| tonic::Status::internal(format!("federation.join prelude serialize: {e}")))?;

    let request = signed_prelude_request(signer, caller_ura, "federation.join", arguments).await?;

    match client.invoke(request).await {
        Ok(reply) => {
            let body_bytes = reply.into_inner().result;
            let projection =
                apply_federation_join_receipt(&body_bytes, authority_published_abilities)?;
            if projection.seeded_ability_count > 0 {
                let ability_count = projection.seeded_ability_count;
                let authority_abilities_revision = projection.authority_abilities_revision;
                crate::op_event!(
                    component = session,
                    kind = authority_broadcast_abilities_seeded,
                    ability_count = ability_count,
                    authority_abilities_revision = authority_abilities_revision,
                );
            }
            Ok(())
        }
        Err(status)
            if status.code() == tonic::Code::AlreadyExists
                || status.message().contains("already") =>
        {
            Ok(())
        }
        Err(status) => Err(status),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FederationJoinReceiptProjection {
    seeded_ability_count: usize,
    authority_abilities_revision: u64,
}

fn apply_federation_join_receipt(
    body_bytes: &[u8],
    authority_published_abilities: &AuthorityPublishedAbilityStore,
) -> Result<FederationJoinReceiptProjection, tonic::Status> {
    if body_bytes.is_empty() {
        return Err(tonic::Status::failed_precondition(
            "federation.join receipt body is empty",
        ));
    }
    let body = crate::daemon::federation::client::ability_contract::parse_receipt::<
        crate::daemon::federation::client::ability_contract::JoinReceipt,
    >(body_bytes)
    .map_err(|error| {
        tonic::Status::failed_precondition(format!("federation.join receipt invalid: {error}"))
    })?;
    authority_published_abilities
        .seed_from_snapshot(
            body.authority_abilities_revision,
            body.authority_published_abilities,
        )
        .map_err(|error| {
            tonic::Status::failed_precondition(format!(
                "federation.join Authority-published ability catalog invalid: {error}"
            ))
        })?;
    Ok(FederationJoinReceiptProjection {
        seeded_ability_count: authority_published_abilities.len(),
        authority_abilities_revision: body.authority_abilities_revision,
    })
}

fn federation_join_public_key_hex(signer: &dyn CanonicalSigner) -> Result<String, Status> {
    signer
        .signing_public_key()
        .map(|key| hex::encode(key.to_bytes()))
        .map_err(|err| signing_identity_status("federation.join public key", err))
}

async fn send_advertise_agent_prelude(
    client: &mut InvocationClient<Channel>,
    agent_ura: &str,
    arguments: Vec<u8>,
    signer: &dyn CanonicalSigner,
) -> Result<
    crate::daemon::federation::hosted_agent_publication::HostedAgentGenerationAssignment,
    tonic::Status,
> {
    let request =
        signed_prelude_request(signer, agent_ura, "federation.advertise_agent", arguments).await?;
    let response = invoke_prelude_unary(client, request, "federation.advertise_agent").await?;
    let receipt: crate::daemon::invocation::dispatch::federation_wrappers::AdvertiseAgentResponse =
        serde_json::from_slice(&response.result).map_err(|error| {
            tonic::Status::failed_precondition(format!(
                "federation.advertise_agent assignment invalid: {error}"
            ))
        })?;
    if !receipt.ack {
        return Err(tonic::Status::failed_precondition(
            "federation.advertise_agent did not acknowledge the assignment",
        ));
    }
    receipt.assignment.validate().map_err(|error| {
        tonic::Status::failed_precondition(format!(
            "federation.advertise_agent assignment invalid: {error}"
        ))
    })?;
    Ok(receipt.assignment)
}

async fn send_advertise_abilities_prelude(
    client: &mut InvocationClient<Channel>,
    owner_ura: &str,
    host_device_ura: &str,
    signer: &dyn CanonicalSigner,
    descriptors: &[AbilityDescriptor],
) -> Result<(), tonic::Status> {
    let projection = crate::daemon::federation::read_model::owner_projection::prepare_and_persist(
        owner_ura,
        host_device_ura,
        descriptors,
    )
    .map_err(|e| {
        tonic::Status::internal(format!(
            "federation.advertise_abilities prelude projection: {e}"
        ))
    })?;
    send_prepared_advertise_abilities_prelude(
        client,
        owner_ura,
        signer,
        PreludeOwnerProjectionAuthority::SponsorDevice,
        &projection,
    )
    .await
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum UserServiceAdvertiseAbilitiesPreludeOutcome {
    Published,
    ReadModelRejected {
        accepted_count: usize,
        expected_count: usize,
        outcome: Option<String>,
    },
}

async fn send_user_service_advertise_abilities_prelude(
    client: &mut InvocationClient<Channel>,
    owner_ura: &str,
    host_device_ura: &str,
    device_signer: &dyn CanonicalSigner,
    user_signer: &dyn CanonicalSigner,
    descriptors: &[AbilityDescriptor],
) -> Result<UserServiceAdvertiseAbilitiesPreludeOutcome, tonic::Status> {
    let projection = crate::daemon::federation::read_model::owner_projection::prepare_and_persist(
        owner_ura,
        host_device_ura,
        descriptors,
    )
    .map_err(|e| {
        tonic::Status::internal(format!(
            "federation.advertise_abilities Service prelude projection: {e}"
        ))
    })?;
    let response = invoke_prepared_advertise_abilities_prelude(
        client,
        owner_ura,
        device_signer,
        PreludeOwnerProjectionAuthority::UserDelegation(user_signer),
        &projection,
    )
    .await?;
    classify_user_service_advertise_abilities_response(response, projection.ability_summaries.len())
}

#[derive(Clone, Copy)]
enum PreludeOwnerProjectionAuthority<'a> {
    SponsorDevice,
    UserDelegation(&'a dyn CanonicalSigner),
}

async fn send_prepared_advertise_abilities_prelude(
    client: &mut InvocationClient<Channel>,
    owner_ura: &str,
    device_signer: &dyn CanonicalSigner,
    authority: PreludeOwnerProjectionAuthority<'_>,
    projection: &crate::daemon::federation::read_model::owner_projection::OwnerProjectionPublication,
) -> Result<(), tonic::Status> {
    let response = invoke_prepared_advertise_abilities_prelude(
        client,
        owner_ura,
        device_signer,
        authority,
        projection,
    )
    .await?;
    crate::daemon::federation::advertise::validate_advertise_abilities_response(
        response,
        projection.ability_summaries.len(),
    )
    .map(|_| ())
    .map_err(tonic::Status::failed_precondition)
}

async fn invoke_prepared_advertise_abilities_prelude(
    client: &mut InvocationClient<Channel>,
    owner_ura: &str,
    device_signer: &dyn CanonicalSigner,
    authority: PreludeOwnerProjectionAuthority<'_>,
    projection: &crate::daemon::federation::read_model::owner_projection::OwnerProjectionPublication,
) -> Result<crate::daemon::federation::advertise::AdvertiseAbilitiesResponse, tonic::Status> {
    let body = serde_json::json!({
        "owner_ura": &projection.owner_ura,
        "host_device_ura": &projection.host_device_ura,
        "generation": projection.generation,
        "projection_revision": projection.projection_revision,
        "projection_digest": &projection.projection_digest,
        "lease_expires_unix_ms": projection.lease_expires_unix_ms,
        "ability_summaries": &projection.ability_summaries,
    });
    let arguments = serde_json::to_vec(&body).map_err(|e| {
        tonic::Status::internal(format!(
            "federation.advertise_abilities prelude serialize: {e}"
        ))
    })?;

    let mut request = signed_prelude_request(
        device_signer,
        owner_ura,
        "federation.advertise_abilities",
        arguments,
    )
    .await?;
    attach_owner_projection_authority(&mut request, owner_ura, device_signer, authority).await?;

    let response = invoke_prelude_unary(client, request, "federation.advertise_abilities").await?;
    crate::daemon::federation::advertise::parse_advertise_abilities_response(&response.result)
        .map_err(tonic::Status::failed_precondition)
}

fn classify_user_service_advertise_abilities_response(
    response: crate::daemon::federation::advertise::AdvertiseAbilitiesResponse,
    expected_count: usize,
) -> Result<UserServiceAdvertiseAbilitiesPreludeOutcome, tonic::Status> {
    if response.ack && response.count == expected_count {
        return Ok(UserServiceAdvertiseAbilitiesPreludeOutcome::Published);
    }
    if response.is_read_model_rejection() {
        return Ok(
            UserServiceAdvertiseAbilitiesPreludeOutcome::ReadModelRejected {
                accepted_count: response.count,
                expected_count,
                outcome: response.outcome,
            },
        );
    }
    crate::daemon::federation::advertise::validate_advertise_abilities_response(
        response,
        expected_count,
    )
    .map(|_| UserServiceAdvertiseAbilitiesPreludeOutcome::Published)
    .map_err(tonic::Status::failed_precondition)
}

async fn attach_owner_projection_authority(
    request: &mut InvokeRequest,
    owner_ura: &str,
    device_signer: &dyn CanonicalSigner,
    authority: PreludeOwnerProjectionAuthority<'_>,
) -> Result<(), tonic::Status> {
    let issuer_signer = match authority {
        PreludeOwnerProjectionAuthority::SponsorDevice => device_signer,
        PreludeOwnerProjectionAuthority::UserDelegation(user_signer) => user_signer,
    };
    let now_ms = i64::try_from(crate::daemon::invocation::admission::runtime_trust::now_unix_ms())
        .map_err(|_| tonic::Status::internal("runtime clock exceeded signed delegation range"))?;
    let hub_ura = session_hub_ura(device_signer.owner_ura())?;
    let claims = crate::daemon::ability::DelegationAuthorityClaims::new(
        issuer_signer.owner_ura(),
        owner_ura,
        device_signer.owner_ura(),
        &hub_ura,
        [crate::daemon::ability::conformance::ABILITY_FEDERATION_ADVERTISE_ABILITIES],
        now_ms,
        now_ms + 60_000,
    )
    .map_err(|error| {
        tonic::Status::failed_precondition(format!(
            "federation.advertise_abilities delegation claims: {error}"
        ))
    })?;
    let metadata = claims
        .signed_metadata_value(issuer_signer)
        .await
        .map_err(|error| {
            tonic::Status::failed_precondition(format!(
                "federation.advertise_abilities delegation signing: {error}"
            ))
        })?;
    request.metadata.insert(
        crate::daemon::ability::RUNTIME_DELEGATION_METADATA_KEY.to_string(),
        metadata,
    );
    Ok(())
}

pub(super) async fn signed_prelude_request(
    signer: &dyn CanonicalSigner,
    subject_ura: &str,
    function_name: &str,
    arguments: Vec<u8>,
) -> Result<InvokeRequest, Status> {
    let caller_ura = signer.owner_ura();
    let hub_ura = session_hub_ura(caller_ura)?;
    let descriptor_subject_ura =
        descriptor_prelude_subject_ura(&hub_ura, subject_ura, function_name)?;
    let descriptor_ref =
        crate::daemon::axon_bridge::descriptor_ref::system_protocol_descriptor_ref_for_wire(
            &hub_ura,
            function_name,
            crate::daemon::ability::CallMode::Rpc,
        )
        .map_err(|err| {
            Status::internal(format!(
                "{function_name} prelude signing requires an explicit descriptor ref: {err}"
            ))
        })?;
    crate::daemon::invocation::ProtoEnvelope::from_target(
        caller_ura,
        hub_ura,
        descriptor_subject_ura,
        crate::daemon::invocation::RootInvocationDerivationIssuer::fresh_root(),
    )
    .map_err(|error| Status::invalid_argument(format!("{function_name} prelude: {error}")))?
    .signed_descriptor_ref_invoke_request_with_signer(
        function_name,
        descriptor_ref,
        arguments,
        signer,
    )
    .await
    .map_err(|error| Status::failed_precondition(format!("{function_name} prelude: {error}")))
}

fn signing_identity_status(
    operation: &str,
    error: crate::daemon::identity::self_identity::SelfIdentityError,
) -> Status {
    Status::failed_precondition(format!(
        "{operation} requires the bound daemon signing identity: {error}"
    ))
}

fn descriptor_prelude_subject_ura(
    hub_ura: &str,
    subject_ura: &str,
    function_name: &str,
) -> Result<String, Status> {
    if crate::daemon::invocation::dispatch::invocation_wire::try_entity_ref(subject_ura.to_string())
        .is_ok()
    {
        return Ok(subject_ura.to_string());
    }
    crate::core::ura::owner_ability_ura(hub_ura, function_name).ok_or_else(|| {
        Status::invalid_argument(format!(
            "{function_name} prelude: subject `{subject_ura}` is not descriptor-bound \
             and hub `{hub_ura}` cannot own the ability"
        ))
    })
}

fn session_hub_ura(caller_ura: &str) -> Result<String, Status> {
    crate::core::ura::parse_ura(caller_ura)
        .map(|parsed| crate::core::ura::hub_ura(&parsed.realm))
        .map_err(|err| {
            Status::invalid_argument(format!(
                "session prelude caller URA `{caller_ura}` is invalid: {err}"
            ))
        })
}

#[derive(Clone)]
pub struct UserTrustSync {
    pub daemon_realm: String,
    pub trust_anchor_path: PathBuf,
    pub cell: crate::daemon::trust::cell::SharedTrustAnchor,
    pub user_signer: PairedUserTrustSigner,
}

/// Narrow signer source for paired User trust bootstrap.
///
/// Device session preludes normally sign as the Device. User trust publication
/// is different: `identity.register_pubkey` for a User trust row must be
/// authored by the User caller. Keeping this as a small source object avoids
/// leaking the key-service port into the whole session supervisor while making
/// tests inject an explicit User signer.
#[derive(Clone)]
pub struct PairedUserTrustSigner {
    source: PairedUserTrustSignerSource,
}

#[derive(Clone)]
enum PairedUserTrustSignerSource {
    RuntimeCaller,
    #[cfg(test)]
    Fixed(Arc<dyn CanonicalSigner>),
}

impl PairedUserTrustSigner {
    #[must_use]
    pub fn runtime_caller() -> Self {
        Self {
            source: PairedUserTrustSignerSource::RuntimeCaller,
        }
    }

    #[cfg(test)]
    pub(crate) fn fixed(signer: Arc<dyn CanonicalSigner>) -> Self {
        Self {
            source: PairedUserTrustSignerSource::Fixed(signer),
        }
    }

    async fn load(&self, user_ura: &str) -> Result<Arc<dyn CanonicalSigner>, SelfIdentityError> {
        match &self.source {
            PairedUserTrustSignerSource::RuntimeCaller => {
                let user_ura = user_ura.to_string();
                tokio::task::spawn_blocking(move || {
                    crate::daemon::identity::self_identity::load_runtime_caller_signer(user_ura)
                })
                .await
                .map_err(|error| {
                    SelfIdentityError::Transport(format!(
                        "paired user signer loader task failed: {error}"
                    ))
                })?
            }
            #[cfg(test)]
            PairedUserTrustSignerSource::Fixed(signer) => {
                if signer.owner_ura() != user_ura {
                    return Err(SelfIdentityError::Rejected {
                        kind: "policy".into(),
                        message: format!(
                            "fixed paired user signer owner `{}` does not match `{user_ura}`",
                            signer.owner_ura()
                        ),
                    });
                }
                Ok(Arc::clone(signer))
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UserTrustBootstrapError {
    #[error("paired user credentials are unavailable for trust bootstrap: {message}")]
    CredentialsUnavailable { message: String },

    #[error("publishing paired user key for `{user_ura}` failed: {status}")]
    PublishFailed {
        user_ura: String,
        status: tonic::Status,
    },

    #[error("paired user runtime signer for `{user_ura}` is unavailable: {message}")]
    SignerUnavailable { user_ura: String, message: String },

    #[error("paired user `{user_ura}` has no public key registered at the Hub")]
    MissingAtHub { user_ura: String },

    #[error("Hub resolve_key for paired user `{user_ura}` failed: {status}")]
    ResolveFailed {
        user_ura: String,
        status: tonic::Status,
    },

    #[error("importing Hub-attested paired user key for `{user_ura}` failed: {status}")]
    ImportFailed {
        user_ura: String,
        status: tonic::Status,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum UserTrustBootstrapOutcome {
    NotRequired,
    Imported { user_ura: String, key_count: usize },
}

const USER_TRUST_RESYNC_INTERVAL: Duration = Duration::from_secs(60);

fn log_user_trust_bootstrap_outcome(outcome: &UserTrustBootstrapOutcome) {
    match outcome {
        UserTrustBootstrapOutcome::NotRequired => {
            crate::op_event!(
                component = session,
                kind = user_trust_bootstrap_not_required,
            );
        }
        UserTrustBootstrapOutcome::Imported {
            user_ura,
            key_count,
        } => {
            let key_count = *key_count as u64;
            crate::op_event!(
                component = session,
                kind = user_trust_bootstrap_imported,
                user_ura = user_ura,
                key_count = key_count,
            );
        }
    }
}

async fn sync_realm_hub_trust_prelude(
    client: &mut InvocationClient<Channel>,
    signer: &dyn CanonicalSigner,
    sync: &UserTrustSync,
) {
    let caller_ura = signer.owner_ura();
    let Ok(parsed_caller) = crate::core::ura::parse_ura(caller_ura) else {
        return;
    };
    if parsed_caller.realm.as_str() != sync.daemon_realm.as_str() {
        return;
    }

    let hub_ura = crate::core::ura::hub_ura(&sync.daemon_realm);
    let args = match serde_json::to_vec(&serde_json::json!({ "agent_ura": hub_ura })) {
        Ok(v) => v,
        Err(_) => return,
    };
    let request = match signed_prelude_request(
        signer,
        &hub_ura,
        crate::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_RESOLVE_KEY,
        args,
    )
    .await
    {
        Ok(req) => req,
        Err(_) => return,
    };
    let response = match invoke_prelude_unary(client, request, "federation.resolve_key").await {
        Ok(resp) => resp,
        Err(status) => {
            let code = status.code();
            let msg = status.message();
            crate::op_event!(
                component = session,
                kind = hub_trust_sync_resolve_failed,
                code = code,
                error = msg,
                hub_ura = hub_ura,
            );
            return;
        }
    };

    let pubkeys = match resolved_public_keys(&response.result) {
        Ok(pubkeys) => pubkeys,
        Err(error) => {
            crate::op_event!(
                component = session,
                kind = hub_trust_sync_resolve_schema_invalid,
                error = error.to_string(),
                hub_ura = hub_ura,
            );
            return;
        }
    };
    if pubkeys.is_empty() {
        crate::op_event!(
            component = session,
            kind = hub_trust_sync_resolve_empty,
            hub_ura = hub_ura,
            message = "hub returned no hub keys — retaining existing local hub trust anchor",
        );
        return;
    }

    for pubkey_b64 in pubkeys {
        let register_args =
            match RegisterPubkeyRequest::new(hub_ura.as_str(), pubkey_b64, TrustAnchorRole::Hub)
                .to_arguments_bytes()
            {
                Ok(v) => v,
                Err(_) => continue,
            };
        match crate::daemon::invocation::admission::register_device_pubkey::handle(
            &register_args,
            &sync.daemon_realm,
            &sync.trust_anchor_path,
            &sync.cell,
        ) {
            Ok(_) => {
                crate::op_event!(
                    component = session,
                    kind = hub_trust_sync_ok,
                    hub_ura = hub_ura,
                );
            }
            Err(status) if status.code() == tonic::Code::AlreadyExists => {
                crate::op_event!(
                    component = session,
                    kind = hub_trust_sync_already_present,
                    hub_ura = hub_ura,
                );
            }
            Err(status) => {
                let code = status.code();
                let msg = status.message();
                crate::op_event!(
                    component = session,
                    kind = hub_trust_sync_write_failed,
                    code = code,
                    error = msg,
                    hub_ura = hub_ura,
                );
            }
        }
    }
}

async fn sync_paired_user_trust_prelude(
    client: &mut InvocationClient<Channel>,
    sync: &UserTrustSync,
) -> Result<UserTrustBootstrapOutcome, UserTrustBootstrapError> {
    let Some(creds) =
        crate::daemon::persistence::config::load_credentials_optional().map_err(|error| {
            UserTrustBootstrapError::CredentialsUnavailable {
                message: format!("load paired credentials: {error:#}"),
            }
        })?
    else {
        return Ok(UserTrustBootstrapOutcome::NotRequired);
    };
    let user_ura = match creds.runtime_user_binding().map_err(|error| {
        UserTrustBootstrapError::CredentialsUnavailable {
            message: format!("project runtime user binding: {error:#}"),
        }
    })? {
        crate::daemon::persistence::config::RuntimeUserBinding::Bound { user_ura } => user_ura,
        crate::daemon::persistence::config::RuntimeUserBinding::Unbound { .. } => {
            return Ok(UserTrustBootstrapOutcome::NotRequired);
        }
    };
    let realm = creds.realm.trim();
    if realm != sync.daemon_realm {
        return Ok(UserTrustBootstrapOutcome::NotRequired);
    }
    let user_signer = sync.user_signer.load(&user_ura).await.map_err(|error| {
        UserTrustBootstrapError::SignerUnavailable {
            user_ura: user_ura.clone(),
            message: error.to_string(),
        }
    })?;
    let signer_public_key_b64 =
        paired_user_signer_public_key_b64(user_signer.as_ref()).map_err(|error| {
            UserTrustBootstrapError::SignerUnavailable {
                user_ura: user_ura.clone(),
                message: error.to_string(),
            }
        })?;
    let local_public_keys = paired_user_public_keys(sync, &user_ura);
    let publish_public_keys = paired_user_publish_public_keys(&signer_public_key_b64);
    publish_paired_user_keys_prelude(
        client,
        user_signer.as_ref(),
        &user_ura,
        &publish_public_keys,
    )
    .await?;

    let resolve_inputs =
        paired_user_resolve_public_keys(&signer_public_key_b64, &local_public_keys);
    let mut pubkeys = Vec::new();
    for presented_pubkey_b64 in &resolve_inputs {
        let args =
            paired_user_resolve_key_args(&user_ura, presented_pubkey_b64).map_err(|err| {
                UserTrustBootstrapError::ResolveFailed {
                    user_ura: user_ura.clone(),
                    status: tonic::Status::internal(format!(
                        "federation.resolve_key user args encode failed: {err}"
                    )),
                }
            })?;
        let request = match signed_prelude_request(
            user_signer.as_ref(),
            &user_ura,
            crate::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_RESOLVE_KEY,
            args,
        )
        .await
        {
            Ok(req) => req,
            Err(status) => {
                return Err(UserTrustBootstrapError::ResolveFailed { user_ura, status });
            }
        };
        let response = match invoke_prelude_unary(client, request, "federation.resolve_key").await {
            Ok(resp) => resp,
            // A locally-trusted key the hub does not know is stale local
            // state (hub-side cap eviction or debris from an earlier
            // pairing), not a session-fatal condition. Only the active
            // signer key is required to resolve; without this tolerance a
            // single stale trust-anchor row keeps the device offline
            // forever.
            Err(status)
                if status.code() == tonic::Code::NotFound
                    && presented_pubkey_b64 != &signer_public_key_b64 =>
            {
                crate::op_event!(
                    component = session,
                    kind = user_trust_sync_stale_local_key_skipped,
                    user_ura = user_ura,
                    presented_pubkey_b64 = presented_pubkey_b64.as_str(),
                );
                continue;
            }
            Err(status) => {
                let code = status.code();
                let msg = status.message();
                crate::op_event!(
                    component = session,
                    kind = user_trust_sync_resolve_failed,
                    code = code,
                    error = msg,
                    user_ura = user_ura,
                );
                return Err(UserTrustBootstrapError::ResolveFailed { user_ura, status });
            }
        };
        let resolved = resolved_public_keys(&response.result).map_err(|error| {
            UserTrustBootstrapError::ResolveFailed {
                user_ura: user_ura.clone(),
                status: tonic::Status::failed_precondition(format!(
                    "federation.resolve_key user response schema invalid: {error}"
                )),
            }
        })?;
        pubkeys.extend(resolved);
    }
    if pubkeys.is_empty() {
        crate::op_event!(
            component = session,
            kind = user_trust_sync_resolve_empty,
            user_ura = user_ura,
            message = "hub returned no user keys — user key not registered at hub yet",
        );
        return Err(UserTrustBootstrapError::MissingAtHub { user_ura });
    }

    let mut accepted_key_count = 0_usize;
    let mut last_import_error = None;
    for pubkey_b64 in pubkeys {
        let register_args =
            match RegisterPubkeyRequest::new(user_ura.as_str(), pubkey_b64, TrustAnchorRole::User)
                .to_arguments_bytes()
            {
                Ok(v) => v,
                Err(_) => continue,
            };
        match crate::daemon::invocation::admission::register_device_pubkey::handle_protecting(
            &register_args,
            &sync.daemon_realm,
            &sync.trust_anchor_path,
            &sync.cell,
            Some(signer_public_key_b64.as_str()),
        ) {
            Ok(_) => {
                accepted_key_count += 1;
                crate::op_event!(
                    component = session,
                    kind = user_trust_sync_ok,
                    user_ura = user_ura,
                );
            }
            Err(status) if status.code() == tonic::Code::AlreadyExists => {
                accepted_key_count += 1;
                crate::op_event!(
                    component = session,
                    kind = user_trust_sync_already_present,
                    user_ura = user_ura,
                );
            }
            Err(status) => {
                let code = status.code();
                let msg = status.message();
                crate::op_event!(
                    component = session,
                    kind = user_trust_sync_write_failed,
                    code = code,
                    error = msg,
                    user_ura = user_ura,
                );
                last_import_error = Some(status);
            }
        }
    }
    if paired_user_trust_present(sync, &user_ura) {
        return Ok(UserTrustBootstrapOutcome::Imported {
            user_ura,
            key_count: accepted_key_count,
        });
    }
    Err(UserTrustBootstrapError::ImportFailed {
        user_ura,
        status: last_import_error.unwrap_or_else(|| {
            tonic::Status::failed_precondition(
                "federation.resolve_key returned no importable paired user keys",
            )
        }),
    })
}

fn resolved_public_keys(result: &[u8]) -> anyhow::Result<Vec<String>> {
    let parsed = serde_json::from_slice::<serde_json::Value>(result)
        .map_err(|err| anyhow::anyhow!("resolve_key_response_json_invalid: {err}"))?;
    let public_keys = parsed
        .get("public_keys_b64")
        .ok_or_else(|| anyhow::anyhow!("resolve_key_response_missing_public_keys_b64"))?
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("resolve_key_response_public_keys_b64_not_array"))?;
    let mut out = Vec::with_capacity(public_keys.len());
    for (index, value) in public_keys.iter().enumerate() {
        let key = value.as_str().ok_or_else(|| {
            anyhow::anyhow!("resolve_key_response_public_keys_b64[{index}]_not_string")
        })?;
        let key = key.trim();
        if key.is_empty() {
            anyhow::bail!("resolve_key_response_public_keys_b64[{index}]_empty");
        }
        out.push(key.to_string());
    }
    Ok(out)
}

fn paired_user_resolve_key_args(
    user_ura: &str,
    presented_pubkey_b64: &str,
) -> anyhow::Result<Vec<u8>> {
    let presented_pubkey_b64 = presented_pubkey_b64.trim();
    if presented_pubkey_b64.is_empty() {
        anyhow::bail!("paired_user_resolve_key_presented_pubkey_empty");
    }
    crate::daemon::federation::wire_contract::ResolveKeyRequest::new(user_ura)
        .with_presented_pubkey_b64(presented_pubkey_b64)
        .to_arguments_bytes()
        .map_err(Into::into)
}

fn paired_user_trust_present(sync: &UserTrustSync, user_ura: &str) -> bool {
    !paired_user_public_keys(sync, user_ura).is_empty()
}

fn paired_user_public_keys(sync: &UserTrustSync, user_ura: &str) -> Vec<String> {
    sync.cell
        .snapshot()
        .lookup_user_all(user_ura)
        .iter()
        .map(|entry| entry.public_key_b64.trim())
        .filter(|key| !key.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn paired_user_signer_public_key_b64(
    signer: &dyn CanonicalSigner,
) -> Result<String, SelfIdentityError> {
    signer
        .signing_public_key()
        .map(|key| BASE64_STANDARD.encode(key.to_bytes()))
}

fn paired_user_publish_public_keys(signer_public_key_b64: &str) -> Vec<String> {
    vec![signer_public_key_b64.trim().to_string()]
}

fn paired_user_resolve_public_keys(
    signer_public_key_b64: &str,
    local_public_keys: &[String],
) -> Vec<String> {
    let mut keys = Vec::with_capacity(local_public_keys.len() + 1);
    push_unique_public_key(&mut keys, signer_public_key_b64);
    for public_key in local_public_keys {
        push_unique_public_key(&mut keys, public_key);
    }
    keys
}

fn push_unique_public_key(keys: &mut Vec<String>, public_key_b64: &str) {
    let public_key = public_key_b64.trim();
    if !public_key.is_empty() && !keys.iter().any(|existing| existing == public_key) {
        keys.push(public_key.to_string());
    }
}

async fn publish_paired_user_keys_prelude(
    client: &mut InvocationClient<Channel>,
    signer: &dyn CanonicalSigner,
    user_ura: &str,
    public_keys_b64: &[String],
) -> Result<(), UserTrustBootstrapError> {
    for public_key_b64 in public_keys_b64 {
        let args = RegisterPubkeyRequest::new(user_ura, public_key_b64, TrustAnchorRole::User)
            .to_arguments_bytes()
            .map_err(|err| UserTrustBootstrapError::PublishFailed {
                user_ura: user_ura.to_string(),
                status: tonic::Status::internal(format!(
                    "identity.register_pubkey user args encode failed: {err}"
                )),
            })?;
        let request = signed_prelude_request(
            signer,
            user_ura,
            crate::daemon::invocation::admission::register_device_pubkey::ABILITY_IDENTITY_REGISTER_PUBKEY,
            args,
        )
        .await
        .map_err(|status| UserTrustBootstrapError::PublishFailed {
            user_ura: user_ura.to_string(),
            status,
        })?;
        match invoke_prelude_unary(client, request, "identity.register_pubkey").await {
            Ok(_) => {}
            Err(status) if status.code() == tonic::Code::AlreadyExists => {}
            Err(status) => {
                crate::op_event!(
                    component = session,
                    kind = user_trust_sync_publish_failed,
                    code = status.code(),
                    error = status.message(),
                    user_ura = user_ura,
                );
                return Err(UserTrustBootstrapError::PublishFailed {
                    user_ura: user_ura.to_string(),
                    status,
                });
            }
        }
    }
    crate::op_event!(
        component = session,
        kind = user_trust_sync_published,
        user_ura = user_ura,
        key_count = public_keys_b64.len() as u64,
    );
    Ok(())
}

pub(super) async fn invoke_prelude_unary(
    client: &mut InvocationClient<Channel>,
    request: axon_sdk::pb::axon::v1::InvokeRequest,
    ability_name: &str,
) -> Result<axon_sdk::pb::axon::v1::InvokeResponse, tonic::Status> {
    let response = client.invoke(request).await?.into_inner();
    if let Some(error) = response.error.as_ref() {
        let message = if error.code.is_empty() {
            error.message.clone()
        } else if error.message.is_empty() {
            error.code.clone()
        } else {
            format!("{}: {}", error.code, error.message)
        };
        return Err(tonic::Status::failed_precondition(format!(
            "{ability_name} prelude rejected: {message}"
        )));
    }
    Ok(response)
}

#[cfg(test)]
pub(super) fn committed_owner_ability_descriptors(
    descriptors: &[AbilityDescriptor],
    owner_ura: &str,
    host_node_id: Option<&str>,
) -> Vec<AbilityDescriptor> {
    descriptors
        .iter()
        .filter(|descriptor| descriptor.owner_ura == owner_ura)
        .cloned()
        .map(|descriptor| match host_node_id {
            Some(node_id) => descriptor.with_metadata_entry("host_node_id", node_id.to_string()),
            None => descriptor,
        })
        .collect()
}

/// Partition the committed device-native namespace by its real public owner.
///
/// DeviceProfileProjection rows remain a same-device migration cursor. Every
/// executable daemon-native family is published under the device-sponsored
/// SystemAgent that owns/can receive it; the Device is carried separately as
/// `host_device_ura`. User-owned hosted Agents are intentionally excluded and
/// use the identity + ability publication transaction below.
pub(super) fn committed_device_native_owner_descriptors(
    descriptors: &[AbilityDescriptor],
    host_device_ura: &str,
) -> BTreeMap<String, Vec<AbilityDescriptor>> {
    let Ok(host) = crate::core::ura::parse_ura(host_device_ura) else {
        return BTreeMap::new();
    };
    if host.kind != crate::core::ura::URAKind::Device {
        return BTreeMap::new();
    }
    let Some(host_device_id) = host.device_id() else {
        return BTreeMap::new();
    };

    let mut by_owner = BTreeMap::<String, Vec<AbilityDescriptor>>::new();
    for descriptor in descriptors {
        let owner_ura = descriptor.owner_ura.as_str();
        let is_same_device_profile = owner_ura == host_device_ura;
        let is_sponsored_system_agent = crate::core::ura::parse_ura(owner_ura)
            .ok()
            .filter(|owner| owner.realm == host.realm)
            .is_some_and(|owner| {
                owner
                    .device_agent_ids()
                    .is_some_and(|(device_id, system_agent_id)| {
                        device_id == host_device_id
                            && crate::daemon::ability::catalog::profiles::is_declared_daemon_native_system_agent_id(
                                system_agent_id,
                            )
                    })
            });
        if is_same_device_profile || is_sponsored_system_agent {
            by_owner
                .entry(owner_ura.to_string())
                .or_default()
                .push(descriptor.clone());
        }
    }
    by_owner
}

/// Partition committed user-scoped Service descriptors by their public owner.
///
/// A Service is accountable to a User Principal while a Device hosts the
/// executable implementation. It therefore cannot use the device-native
/// `SponsorDevice` publication path; the advertise prelude must carry a
/// paired User delegation over the Service owner URA.
pub(super) fn committed_user_service_owner_descriptors(
    descriptors: &[AbilityDescriptor],
    user_ura: &str,
) -> BTreeMap<String, Vec<AbilityDescriptor>> {
    let Ok(user) = crate::core::ura::parse_ura(user_ura) else {
        return BTreeMap::new();
    };
    if user.kind != crate::core::ura::URAKind::User {
        return BTreeMap::new();
    }
    let Some(user_id) = user.user_id() else {
        return BTreeMap::new();
    };

    let mut by_owner = BTreeMap::<String, Vec<AbilityDescriptor>>::new();
    for descriptor in descriptors {
        let owner_ura = descriptor.owner_ura.as_str();
        let is_user_service = crate::core::ura::parse_ura(owner_ura)
            .ok()
            .filter(|owner| owner.realm == user.realm)
            .and_then(|owner| {
                owner.service_ids().map(|(principal_id, _service_id)| {
                    owner.kind == crate::core::ura::URAKind::Service && principal_id == user_id
                })
            })
            .unwrap_or(false);
        if is_user_service {
            by_owner
                .entry(owner_ura.to_string())
                .or_default()
                .push(descriptor.clone());
        }
    }
    by_owner
}

fn is_service_owner_ura(owner_ura: &str) -> bool {
    crate::core::ura::parse_ura(owner_ura)
        .ok()
        .is_some_and(|owner| owner.kind == crate::core::ura::URAKind::Service)
}

#[cfg(test)]
mod tests {
    use super::{
        apply_federation_join_receipt, attach_owner_projection_authority,
        classify_user_service_advertise_abilities_response,
        committed_user_service_owner_descriptors, paired_user_publish_public_keys,
        paired_user_resolve_key_args, paired_user_resolve_public_keys,
        paired_user_signer_public_key_b64, paired_user_trust_present,
        resolve_hosted_agent_user_segment, resolved_public_keys,
        run_hosted_agent_advertise_prelude, signed_prelude_request, sync_paired_user_trust_prelude,
        PairedUserTrustSigner, PreludeOwnerProjectionAuthority, RegisterPubkeyRequest,
        UserServiceAdvertiseAbilitiesPreludeOutcome, UserTrustBootstrapError,
        UserTrustBootstrapOutcome, UserTrustSync,
    };
    use crate::daemon::ability::descriptors::{AbilityDescriptor, AdmissionAction, Visibility};
    use crate::daemon::federation::client::ability_contract::AuthorityAbilityEntry;
    use crate::daemon::federation::read_model::authority_published_abilities::AuthorityPublishedAbilityStore;
    use crate::daemon::identity::self_identity::TestCanonicalSigner;
    use crate::daemon::persistence::config::{save_credentials, state_dir, Credentials};
    use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustAnchorRole, TrustedAgent};
    use crate::daemon::trust::cell::SharedTrustAnchor;
    use axon_sdk::pb::axon::v1::{invocation_client::InvocationClient, InvokeRequest};
    use base64::Engine as _;
    use std::sync::Arc;
    use tonic::transport::Channel;

    fn canonical_authority_entry(name: &str) -> AuthorityAbilityEntry {
        AuthorityAbilityEntry {
            name: name.to_string(),
            descriptor: serde_json::to_value(
                AbilityDescriptor::new(
                    name,
                    crate::core::ura::hub_ura("realm"),
                    Visibility::Public,
                    AdmissionAction::Read,
                )
                .expect("canonical realm Authority descriptor"),
            )
            .expect("descriptor json"),
        }
    }

    #[test]
    fn resolved_public_keys_prefers_array_response() {
        let body = br#"{
            "public_key_b64": "fallback",
            "public_keys_b64": [" key-a ", "key-b"]
        }"#;

        assert_eq!(
            resolved_public_keys(body).expect("schema-bound public keys"),
            vec!["key-a".to_string(), "key-b".to_string()]
        );
    }

    #[test]
    fn resolved_public_keys_rejects_legacy_single_key_response() {
        let body = br#"{ "public_key_b64": " single-key " }"#;

        let err = resolved_public_keys(body)
            .expect_err("legacy single-key resolve_key response must not be repaired");
        assert!(
            err.to_string()
                .contains("resolve_key_response_missing_public_keys_b64"),
            "{err:#}"
        );
    }

    #[test]
    fn resolved_public_keys_rejects_malformed_rows() {
        assert!(resolved_public_keys(br#"{"public_keys_b64":[]}"#)
            .expect("empty canonical key array is a hub miss")
            .is_empty());
        let json_err = resolved_public_keys(br#"not-json"#)
            .expect_err("malformed JSON must be a schema error");
        assert!(
            json_err
                .to_string()
                .contains("resolve_key_response_json_invalid"),
            "{json_err:#}"
        );
        let row_err = resolved_public_keys(br#"{"public_keys_b64":["ok",7]}"#)
            .expect_err("malformed public_keys_b64 rows must fail closed");
        assert!(
            row_err
                .to_string()
                .contains("resolve_key_response_public_keys_b64[1]_not_string"),
            "{row_err:#}"
        );
        let empty_err = resolved_public_keys(br#"{"public_keys_b64":[" "]}"#)
            .expect_err("empty public_keys_b64 rows must fail closed");
        assert!(
            empty_err
                .to_string()
                .contains("resolve_key_response_public_keys_b64[0]_empty"),
            "{empty_err:#}"
        );
    }

    #[test]
    fn federation_join_receipt_rejects_empty_or_malformed_body() {
        let store = AuthorityPublishedAbilityStore::new();
        let empty = apply_federation_join_receipt(&[], &store)
            .expect_err("empty join receipt must fail closed");
        assert_eq!(empty.code(), tonic::Code::FailedPrecondition);
        assert!(empty.message().contains("receipt body is empty"));

        let malformed = apply_federation_join_receipt(br#"{"unexpected":"shape"}"#, &store)
            .expect_err("malformed join receipt must fail closed");
        assert_eq!(malformed.code(), tonic::Code::FailedPrecondition);
        assert!(malformed
            .message()
            .contains("federation.join receipt invalid"));
    }

    #[test]
    fn federation_join_receipt_seeds_canonical_authority_catalog() {
        let store = AuthorityPublishedAbilityStore::new();
        let body = serde_json::to_vec(&serde_json::json!({
            "membership_ura": "easynet:///r/realm/device/n1",
            "realm": "realm",
            "join_receipt_hash": "a".repeat(64),
            "authority_abilities_revision": 17,
            "authority_published_abilities": [canonical_authority_entry("test.scope")],
            "advertise_contract": {
                "allowed_owner_prefixes": ["device."],
                "allows_hosted_agents": true
            }
        }))
        .expect("join receipt json");

        let projection =
            apply_federation_join_receipt(&body, &store).expect("canonical join receipt");

        assert_eq!(projection.seeded_ability_count, 1);
        assert_eq!(projection.authority_abilities_revision, 17);
        assert_eq!(store.revision(), 17);
        assert_eq!(store.snapshot()[0].public_name(), "test.scope");
    }

    #[test]
    fn user_service_owner_projection_is_partitioned_for_user_delegation() {
        let user_ura = "easynet:///r/realm/user/user-dev";
        let pages_service = crate::core::ura::service_ura("realm", "user-dev", "pages");
        let foreign_service = crate::core::ura::service_ura("realm", "other-user", "pages");
        let system_agent = crate::core::ura::device_agent_ura("realm", "n1", "runtime-health");
        let descriptors = vec![
            AbilityDescriptor::new(
                "project_list",
                &pages_service,
                Visibility::Scoped,
                AdmissionAction::Read,
            )
            .expect("Pages Service descriptor"),
            AbilityDescriptor::new(
                "project_list",
                &foreign_service,
                Visibility::Scoped,
                AdmissionAction::Read,
            )
            .expect("foreign Service descriptor"),
            AbilityDescriptor::new(
                "observe.health",
                &system_agent,
                Visibility::Scoped,
                AdmissionAction::Read,
            )
            .expect("SystemAgent descriptor"),
        ];

        let by_owner = committed_user_service_owner_descriptors(&descriptors, user_ura);

        assert_eq!(by_owner.len(), 1);
        assert_eq!(by_owner[&pages_service][0].public_name(), "project_list");
        assert!(!by_owner.contains_key(&foreign_service));
        assert!(!by_owner.contains_key(&system_agent));
    }

    #[test]
    fn user_service_owner_projection_requires_canonical_user_ura() {
        let pages_service = crate::core::ura::service_ura("realm", "user-dev", "pages");
        let descriptors = vec![AbilityDescriptor::new(
            "project_list",
            &pages_service,
            Visibility::Scoped,
            AdmissionAction::Read,
        )
        .expect("Pages Service descriptor")];

        let by_owner = committed_user_service_owner_descriptors(&descriptors, "not-a-user-ura");

        assert!(by_owner.is_empty());
    }

    #[test]
    fn user_service_projection_conflict_degrades_instead_of_failing_session() {
        let response = crate::daemon::federation::advertise::AdvertiseAbilitiesResponse {
            ack: false,
            count: 0,
            outcome: Some("rejected_conflict".to_string()),
        };

        let outcome = classify_user_service_advertise_abilities_response(response, 5)
            .expect("read-model conflict is a nonfatal Service projection outcome");

        assert_eq!(
            outcome,
            UserServiceAdvertiseAbilitiesPreludeOutcome::ReadModelRejected {
                accepted_count: 0,
                expected_count: 5,
                outcome: Some("rejected_conflict".to_string()),
            }
        );
    }

    #[test]
    fn user_service_projection_count_mismatch_still_fails_closed() {
        let response = crate::daemon::federation::advertise::AdvertiseAbilitiesResponse {
            ack: true,
            count: 3,
            outcome: Some("updated".to_string()),
        };

        let status = classify_user_service_advertise_abilities_response(response, 5)
            .expect_err("acknowledged partial Service projection must still fail");

        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
        assert!(status.message().contains("count mismatch"));
    }

    fn user_trust_sync_with_key(user_ura: &str) -> UserTrustSync {
        UserTrustSync {
            daemon_realm: "realm".to_string(),
            trust_anchor_path: std::path::PathBuf::from("/tmp/easynet-test-realm-trust.toml"),
            cell: SharedTrustAnchor::new(Arc::new(
                RealmTrustAnchor::from_entries(vec![TrustedAgent {
                    agent_ura: user_ura.to_string(),
                    public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
                    role: TrustAnchorRole::User,
                    added_at_unix_ms: 1_700_000_000_000,
                    origin_realm: None,
                    hub_endpoint: None,
                    tls_ca_pem_path: None,
                }])
                .expect("user anchor"),
            )),
            user_signer: PairedUserTrustSigner::fixed(Arc::new(TestCanonicalSigner::new(
                user_ura, [0x22; 32],
            ))),
        }
    }

    fn paired_credentials(username: Option<&str>) -> Credentials {
        Credentials {
            node_id: "n1".to_string(),
            credential_token: "token".to_string(),
            hub_endpoint: "axon://hub.example:7700".to_string(),
            realm: "realm".to_string(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: username.map(str::to_string),
            user_id: Some("user-dev".to_string()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        }
    }

    fn federation_native_credentials_without_user_binding() -> Credentials {
        Credentials {
            node_id: "n1".to_string(),
            credential_token: String::new(),
            hub_endpoint: "https://hub.example:50443".to_string(),
            realm: "realm".to_string(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: None,
            user_id: None,
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: Some("a".repeat(64)),
        }
    }

    #[test]
    fn hosted_agent_owner_segment_reads_valid_paired_credentials() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        save_credentials(&paired_credentials(Some("dev"))).expect("save credentials");

        let user_segment = resolve_hosted_agent_user_segment("https://hub:50443")
            .expect("credential user segment");

        assert_eq!(user_segment, "user-dev");
    }

    #[test]
    fn hosted_agent_owner_segment_rejects_federation_native_credentials_without_user_binding() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        save_credentials(&federation_native_credentials_without_user_binding())
            .expect("save federation-native device credential");

        let error = resolve_hosted_agent_user_segment("https://hub:50443")
            .expect_err("missing user binding must fail hosted-agent prelude");

        match error {
            crate::daemon::invocation::bidi::session_initiator::SessionError::HostedAgentPreludeFailed {
                reason,
                ..
            } => {
                assert!(
                    reason.contains("project runtime user binding for hosted-agent owner projection"),
                    "{reason}"
                );
                assert!(reason.contains("not bound"), "{reason}");
            }
            other => panic!("expected hosted-agent credential projection failure, got {other:?}"),
        }
    }

    #[test]
    fn paired_user_trust_present_reads_user_key_bucket() {
        let user_ura = "easynet:///r/realm/user/user-dev";
        let sync = user_trust_sync_with_key(user_ura);

        assert!(paired_user_trust_present(&sync, user_ura));
        assert!(!paired_user_trust_present(
            &sync,
            "easynet:///r/realm/user/other"
        ));
    }

    #[tokio::test]
    async fn paired_user_trust_bootstrap_ignores_missing_credentials_only() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let sync = user_trust_sync_with_key("easynet:///r/realm/user/user-dev");
        let mut client =
            InvocationClient::new(Channel::from_static("http://127.0.0.1:1").connect_lazy());

        let outcome = sync_paired_user_trust_prelude(&mut client, &sync)
            .await
            .expect("missing credentials are the only not-required local state");
        assert_eq!(outcome, UserTrustBootstrapOutcome::NotRequired);
    }

    #[tokio::test]
    async fn paired_user_trust_bootstrap_skips_device_only_credentials() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        save_credentials(&federation_native_credentials_without_user_binding())
            .expect("save federation-native device credential");
        let sync = user_trust_sync_with_key("easynet:///r/realm/user/user-dev");
        let mut client =
            InvocationClient::new(Channel::from_static("http://127.0.0.1:1").connect_lazy());

        let outcome = sync_paired_user_trust_prelude(&mut client, &sync)
            .await
            .expect("device-only runtime has no paired-user trust prelude");
        assert_eq!(outcome, UserTrustBootstrapOutcome::NotRequired);
    }

    #[tokio::test]
    async fn hosted_agent_advertise_prelude_skips_device_only_credentials() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        save_credentials(&federation_native_credentials_without_user_binding())
            .expect("save federation-native device credential");
        let mut phase = super::SessionPhaseTracker::new();
        let signer = TestCanonicalSigner::new("easynet:///r/realm/device/n1", [0x11; 32]);
        let mut client =
            InvocationClient::new(Channel::from_static("http://127.0.0.1:1").connect_lazy());

        run_hosted_agent_advertise_prelude(
            &mut client,
            &mut phase,
            "https://hub.example:50443",
            &signer,
            None,
            &[],
        )
        .await
        .expect("device-only runtime must skip user-root hosted-agent publication");
    }

    #[tokio::test]
    async fn paired_user_trust_bootstrap_rejects_malformed_credentials() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        std::fs::create_dir_all(state_dir()).expect("create state dir");
        std::fs::write(state_dir().join("credentials.json"), "{")
            .expect("write malformed credentials");
        let sync = user_trust_sync_with_key("easynet:///r/realm/user/user-dev");
        let mut client =
            InvocationClient::new(Channel::from_static("http://127.0.0.1:1").connect_lazy());

        let err = sync_paired_user_trust_prelude(&mut client, &sync)
            .await
            .expect_err("malformed credentials must fail prelude, not project NotRequired");
        match err {
            UserTrustBootstrapError::CredentialsUnavailable { message } => {
                assert!(message.contains("load paired credentials"), "{message}");
                assert!(message.contains("parse credentials"), "{message}");
            }
            other => panic!("expected credential unavailable state, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn paired_user_register_pubkey_prelude_is_bootstrap_candidate_tuple() {
        let user_ura = "easynet:///r/realm/user/user-dev";
        let signer = TestCanonicalSigner::new(user_ura, [0x33; 32]);
        let public_key_b64 = base64::engine::general_purpose::STANDARD.encode(
            ed25519_dalek::SigningKey::from_bytes(&[0x33; 32])
                .verifying_key()
                .to_bytes(),
        );
        let args = RegisterPubkeyRequest::new(user_ura, public_key_b64, TrustAnchorRole::User)
            .to_arguments_bytes()
            .expect("identity.register_pubkey user args");

        let request = signed_prelude_request(
            &signer,
            user_ura,
            crate::daemon::invocation::admission::register_device_pubkey::ABILITY_IDENTITY_REGISTER_PUBKEY,
            args,
        )
        .await
        .expect("signed identity.register_pubkey prelude request");
        let envelope = request.envelope.as_ref().expect("prelude envelope");

        assert!(
            crate::daemon::invocation::admission::register_device_pubkey::RegisterPubkeyBootstrapTuple::matches(envelope),
            "paired user trust prelude must be classified as a bootstrap candidate before Axon caller-key resolution"
        );
        crate::daemon::invocation::admission::register_device_pubkey::verify_user_register_pubkey_bootstrap_claim(
            envelope,
            &request.arguments,
        )
        .expect("prelude arguments must bind the presented user key");
    }

    #[tokio::test]
    async fn hosted_agent_projection_carries_exact_user_delegation() {
        let user_ura = "easynet:///r/realm/user/user-dev";
        let device_ura = "easynet:///r/realm/device/device-dev";
        let agent_ura = "easynet:///r/realm/agent/user-dev.worker";
        let user_signer = TestCanonicalSigner::new(user_ura, [0x31; 32]);
        let device_signer = TestCanonicalSigner::new(device_ura, [0x32; 32]);
        let mut request = InvokeRequest::default();

        attach_owner_projection_authority(
            &mut request,
            agent_ura,
            &device_signer,
            PreludeOwnerProjectionAuthority::UserDelegation(&user_signer),
        )
        .await
        .expect("hosted Agent projection delegation");

        let raw = request
            .metadata
            .get(crate::daemon::ability::RUNTIME_DELEGATION_METADATA_KEY)
            .expect("signed runtime delegation metadata");
        let wire = crate::daemon::invocation::admission::authority_metadata::decode_delegation_authority_wire(raw)
            .expect("canonical delegation wire");
        assert_eq!(wire.payload.issuer_ura(), user_ura);
        assert_eq!(wire.payload.caller_ura(), device_ura);
        assert_eq!(wire.payload.subject_ura(), agent_ura);
        assert_eq!(wire.payload.audience(), "easynet:///r/realm/authority");
        assert_eq!(
            wire.payload.scopes(),
            [crate::daemon::ability::conformance::ABILITY_FEDERATION_ADVERTISE_ABILITIES]
        );
    }

    #[tokio::test]
    async fn system_agent_projection_carries_sponsor_device_delegation() {
        let device_ura = "easynet:///r/realm/device/device-dev";
        let system_agent_ura = "easynet:///r/realm/agent/device.device-dev.remote-desktop";
        let device_signer = TestCanonicalSigner::new(device_ura, [0x42; 32]);
        let mut request = InvokeRequest::default();

        attach_owner_projection_authority(
            &mut request,
            system_agent_ura,
            &device_signer,
            PreludeOwnerProjectionAuthority::SponsorDevice,
        )
        .await
        .expect("SystemAgent projection sponsor-device delegation");

        let raw = request
            .metadata
            .get(crate::daemon::ability::RUNTIME_DELEGATION_METADATA_KEY)
            .expect("signed sponsor-device runtime delegation metadata");
        let wire =
            crate::daemon::invocation::admission::authority_metadata::decode_delegation_authority_wire(
                raw,
            )
            .expect("canonical delegation wire");
        assert_eq!(wire.payload.issuer_ura(), device_ura);
        assert_eq!(wire.payload.caller_ura(), device_ura);
        assert_eq!(wire.payload.subject_ura(), system_agent_ura);
        assert_eq!(wire.payload.audience(), "easynet:///r/realm/authority");
        assert_eq!(
            wire.payload.scopes(),
            [crate::daemon::ability::conformance::ABILITY_FEDERATION_ADVERTISE_ABILITIES]
        );
    }

    #[test]
    fn paired_user_resolve_key_args_carries_presented_pubkey() {
        let body = paired_user_resolve_key_args(
            "easynet:///r/realm/user/user-dev",
            " AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA= ",
        )
        .expect("encode paired user resolve args");
        let parsed: serde_json::Value = serde_json::from_slice(&body).expect("json");

        assert_eq!(parsed["agent_ura"], "easynet:///r/realm/user/user-dev");
        assert_eq!(
            parsed["presented_pubkey_b64"],
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
        );
    }

    #[test]
    fn paired_user_resolve_key_args_rejects_missing_presented_pubkey() {
        let error = paired_user_resolve_key_args("easynet:///r/realm/user/user-dev", "   ")
            .expect_err("paired user resolve_key must pin the local signer key");

        assert!(
            error
                .to_string()
                .contains("paired_user_resolve_key_presented_pubkey_empty"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn paired_user_trust_bootstrap_publishes_only_current_signer_key() {
        let user_ura = "easynet:///r/realm/user/user-dev";
        let signer = TestCanonicalSigner::new(user_ura, [0x33; 32]);
        let signer_key = paired_user_signer_public_key_b64(&signer).expect("signer key");
        let stale_or_browser_key = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string();

        assert_eq!(
            paired_user_publish_public_keys(&signer_key),
            vec![signer_key.clone()],
            "current signer may only self-register its own public key"
        );
        assert_eq!(
            paired_user_resolve_public_keys(
                &signer_key,
                &[stale_or_browser_key.clone(), signer_key.clone()],
            ),
            vec![signer_key, stale_or_browser_key],
            "non-signer local user keys may be resolved/imported, but must not be signer-published"
        );
    }

    #[test]
    fn hosted_agent_prelude_plan_rejects_empty_runtime_projection() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let owner = "easynet:///r/realm/agent/dev.worker";
        let result =
            crate::daemon::federation::hosted_agent_publication::HostedAgentPublicationPlan::begin(
                owner,
                "easynet:///r/realm/device/n1",
                Some("n1"),
                &[],
            );
        let error = match result {
            Ok(_) => panic!("an empty hosted owner must not be published as online"),
            Err(error) => error,
        };

        assert!(error.contains(owner), "{error}");
        assert!(
            error.contains("no committed LocalRuntime descriptors"),
            "{error}"
        );
    }

    #[test]
    fn hosted_agent_prelude_plan_persists_and_reuses_pending_incarnation() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let owner = "easynet:///r/realm/agent/dev.anthropic";
        let host = "easynet:///r/realm/device/n1";
        crate::daemon::persistence::local_agents::save_test_llm_publication_owner(host, owner)
            .expect("persist explicit local publication ownership");
        let descriptors = vec![AbilityDescriptor::new(
            "chat",
            owner,
            Visibility::Scoped,
            AdmissionAction::Invoke,
        )
        .expect("hosted-agent descriptor")];

        let first =
            crate::daemon::federation::hosted_agent_publication::HostedAgentPublicationPlan::begin(
                owner,
                host,
                Some("n1"),
                &descriptors,
            )
            .expect("first hosted-agent plan");
        let retry =
            crate::daemon::federation::hosted_agent_publication::HostedAgentPublicationPlan::begin(
                owner,
                host,
                Some("n1"),
                &descriptors,
            )
            .expect("retry hosted-agent plan");

        assert_eq!(first.incarnation_id(), retry.incarnation_id());
        assert!(crate::daemon::persistence::owner_projections::load()
            .unwrap()
            .cursor_for(owner)
            .is_none());
    }
}
