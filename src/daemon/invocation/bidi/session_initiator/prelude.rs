use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axon_sdk::pb::axon::v1::{invocation_client::InvocationClient, InvokeRequest};
use tonic::{transport::Channel, Status};

use super::heartbeat::spawn_federation_heartbeat;
use super::supervisor::{DeviceSessionPhase, PreludeStep, SessionPhaseTracker};
use super::tasks::AbortOnDrop;
use super::SessionError;
use crate::daemon::ability::builtins::resources::pages::identity::pages_user_from_env_or_credentials;
use crate::daemon::ability::descriptors::AbilityDescriptor;
use crate::daemon::federation::read_model::authority_published_abilities::AuthorityPublishedAbilityStore;
use crate::daemon::identity::self_identity::CanonicalSigner;
use crate::daemon::invocation::admission::register_device_pubkey::RegisterPubkeyRequest;
use crate::daemon::persistence::agent_aggregate::{
    AgentAggregateRepository, AgentHostedAdvertiseEntry,
};
use crate::daemon::trust::anchor::TrustedAgentRole;

pub struct SessionPreludeInputs<'a> {
    pub(super) ability_descriptors: &'a [AbilityDescriptor],
    pub(super) authority_published_abilities: Arc<AuthorityPublishedAbilityStore>,
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
        }
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
    let owner_descriptors = committed_owner_ability_descriptors(
        ability_descriptors,
        &caller_ura,
        crate::core::ura::parse_ura(&caller_ura)
            .ok()
            .and_then(|parsed| parsed.device_id().map(str::to_string))
            .as_deref(),
    );
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
        &owner_descriptors,
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
    ability_descriptors: &[AbilityDescriptor],
) -> Result<(), SessionError> {
    phase.transition(
        DeviceSessionPhase::Preluding(PreludeStep::OwnerProjection),
        "join_prelude_done",
    );
    if ability_descriptors.is_empty() {
        return Ok(());
    }

    let ability_count = ability_descriptors.len();
    crate::op_event!(
        component = session,
        kind = advertise_abilities_prelude_sending,
        ability_count = ability_count,
    );
    if let Err(status) = send_advertise_abilities_prelude(
        client,
        caller_ura,
        caller_ura,
        signer,
        ability_descriptors,
    )
    .await
    {
        let code = status.code();
        let msg = status.message();
        crate::op_event!(
            component = session,
            kind = advertise_abilities_prelude_failed,
            code = code,
            error = msg,
            message = "owner projection publish failed; reconnecting instead of exposing an online owner with empty abilities",
        );
        return Err(SessionError::OwnerProjectionFailed {
            endpoint: hub_endpoint.to_string(),
            status,
        });
    }

    crate::op_event!(
        component = session,
        kind = advertise_abilities_prelude_ok,
        ability_count = ability_count,
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
    let outcome = sync_paired_user_trust_prelude(client, signer.as_ref(), sync).await?;
    log_user_trust_bootstrap_outcome(&outcome);
    let sync = sync.clone();
    Ok(Some(AbortOnDrop(tokio::spawn(async move {
        let mut resync_client = InvocationClient::new(resync_channel);
        loop {
            tokio::time::sleep(USER_TRUST_RESYNC_INTERVAL).await;
            sync_realm_hub_trust_prelude(&mut resync_client, signer.as_ref(), &sync).await;
            if let Err(err) =
                sync_paired_user_trust_prelude(&mut resync_client, signer.as_ref(), &sync).await
            {
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
    ability_descriptors: &[AbilityDescriptor],
) -> Result<(), SessionError> {
    let caller_ura = signer.owner_ura();
    let realm = crate::core::ura::parse_ura(caller_ura)
        .map(|parsed| parsed.realm)
        .map_err(|error| SessionError::HostedAgentPreludeFailed {
            endpoint: hub_endpoint.to_string(),
            reason: format!("signer owner URA `{caller_ura}` is invalid: {error}"),
        })?;
    // The agent owner-prefix is the USERNAME slug (`<username>.<agent>`, e.g.
    // `dev.pages`), NOT the user UUID. This is the §15.1-3 dual grammar: subject
    // URAs anchor on the stable UUID, but owner-prefixed agent/resource URAs keep
    // the username slug. The backend resolves these owners via
    // `svc.UsernameForUID` (username), so advertising under the UUID
    // (`<uuid>.pages`) lands a directory entry the resolver never queries →
    // `namespace.resolve NXDOMAIN: owner is not online` on `project_list`/etc.
    let user_segment = resolve_hosted_agent_user_segment(hub_endpoint)?;

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
    let credentials =
        crate::daemon::persistence::config::load_credentials_optional().map_err(|error| {
            SessionError::HostedAgentPreludeFailed {
                endpoint: hub_endpoint.to_string(),
                reason: format!("load credentials for hosted-agent owner projection: {error}"),
            }
        })?;
    pages_user_from_env_or_credentials(credentials.as_ref())
    .map_err(|error| SessionError::HostedAgentPreludeFailed {
        endpoint: hub_endpoint.to_string(),
        reason: format!("project username for hosted-agent owner projection: {error}"),
    })?
    .ok_or_else(|| SessionError::HostedAgentPreludeFailed {
        endpoint: hub_endpoint.to_string(),
        reason: "project username for hosted-agent owner projection: no user-root Pages identity is bound".to_string(),
    })
}

async fn advertise_hosted_agent_entry(
    client: &mut InvocationClient<Channel>,
    caller_ura: &str,
    caller_node_id: &Option<String>,
    ability_descriptors: &[AbilityDescriptor],
    entry: &AgentHostedAdvertiseEntry,
    signer: &dyn CanonicalSigner,
) -> Result<(), String> {
    let host_for_advertise = caller_node_id.as_deref();
    let plan = HostedAgentPreludePublicationPlan::prepare(
        entry.agent_ura(),
        caller_ura,
        host_for_advertise,
        ability_descriptors,
    )?;
    send_advertise_agent_prelude(
        client,
        entry.agent_ura(),
        plan.generation(),
        host_for_advertise,
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
    let mut advertise_ctx = HostedAgentAbilityAdvertiseContext { client, signer };
    advertise_hosted_agent_abilities(&mut advertise_ctx, entry, &plan).await
}

struct HostedAgentAbilityAdvertiseContext<'a> {
    client: &'a mut InvocationClient<Channel>,
    signer: &'a dyn CanonicalSigner,
}

struct HostedAgentPreludePublicationPlan {
    descriptors: Vec<AbilityDescriptor>,
    publication:
        crate::daemon::federation::read_model::owner_projection::OwnerProjectionPublication,
}

impl HostedAgentPreludePublicationPlan {
    fn prepare(
        agent_ura: &str,
        host_device_ura: &str,
        host_node_id: Option<&str>,
        ability_descriptors: &[AbilityDescriptor],
    ) -> Result<Self, String> {
        let descriptors =
            committed_owner_ability_descriptors(ability_descriptors, agent_ura, host_node_id);
        let publication =
            crate::daemon::federation::read_model::owner_projection::prepare_and_persist(
                agent_ura,
                host_device_ura,
                &descriptors,
            )?;
        Ok(Self {
            descriptors,
            publication,
        })
    }

    fn generation(&self) -> u64 {
        self.publication.generation
    }

    fn ability_count(&self) -> usize {
        self.descriptors.len()
    }
}

async fn advertise_hosted_agent_abilities(
    ctx: &mut HostedAgentAbilityAdvertiseContext<'_>,
    entry: &AgentHostedAdvertiseEntry,
    plan: &HostedAgentPreludePublicationPlan,
) -> Result<(), String> {
    let ability_count = plan.ability_count();
    crate::op_event!(
        component = session,
        kind = advertise_hosted_agent_abilities_prelude_sending,
        agent_ura = entry.agent_ura(),
        ability_count = ability_count,
    );
    send_prepared_advertise_abilities_prelude(
        ctx.client,
        entry.agent_ura(),
        ctx.signer,
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
    generation: u64,
    host_node_id: Option<&str>,
    signer: &dyn CanonicalSigner,
) -> Result<(), tonic::Status> {
    let caller_ura = signer.owner_ura();
    let mut body = serde_json::json!({
        "agent_ura": agent_ura,
        "generation": generation,
        "signing_authority": {
            "kind": "hosted_by",
            "host_ura": caller_ura,
        },
    });
    if let Some(node_id) = host_node_id {
        if let Some(map) = body.as_object_mut() {
            map.insert(
                "host_node_id".to_string(),
                serde_json::Value::String(node_id.to_string()),
            );
        }
    }
    let arguments = serde_json::to_vec(&body).map_err(|e| {
        tonic::Status::internal(format!("federation.advertise_agent prelude serialize: {e}"))
    })?;

    let request =
        signed_prelude_request(signer, agent_ura, "federation.advertise_agent", arguments).await?;

    invoke_prelude_unary(client, request, "federation.advertise_agent")
        .await
        .map(|_| ())
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
    send_prepared_advertise_abilities_prelude(client, owner_ura, signer, &projection).await
}

async fn send_prepared_advertise_abilities_prelude(
    client: &mut InvocationClient<Channel>,
    owner_ura: &str,
    signer: &dyn CanonicalSigner,
    projection: &crate::daemon::federation::read_model::owner_projection::OwnerProjectionPublication,
) -> Result<(), tonic::Status> {
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

    let request = signed_prelude_request(
        signer,
        owner_ura,
        "federation.advertise_abilities",
        arguments,
    )
    .await?;

    invoke_prelude_unary(client, request, "federation.advertise_abilities")
        .await
        .map(|_| ())
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
        crate::daemon::axon_bridge::descriptor_ref::catalog_descriptor_ref_for_wire(
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
        let register_args = match serde_json::to_vec(&serde_json::json!({
            "agent_ura": hub_ura,
            "public_key_b64": pubkey_b64,
            "role": "hub",
        })) {
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
    signer: &dyn CanonicalSigner,
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
    let user_ura =
        creds
            .user_ura()
            .map_err(|error| UserTrustBootstrapError::CredentialsUnavailable {
                message: format!("project paired user URA: {error:#}"),
            })?;
    let realm = creds.realm.trim();
    if realm != sync.daemon_realm {
        return Ok(UserTrustBootstrapOutcome::NotRequired);
    }
    let local_public_keys = paired_user_public_keys(sync, &user_ura);
    if !local_public_keys.is_empty() {
        publish_paired_user_keys_prelude(client, signer, &user_ura, &local_public_keys).await?;
    }

    let mut resolve_inputs: Vec<Option<&str>> = local_public_keys
        .iter()
        .map(|public_key| Some(public_key.as_str()))
        .collect();
    if resolve_inputs.is_empty() {
        resolve_inputs.push(None);
    }
    let mut pubkeys = Vec::new();
    for presented_pubkey_b64 in resolve_inputs {
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
            signer,
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
        let register_args = match serde_json::to_vec(&serde_json::json!({
            "principal_ura": user_ura,
            "public_key_b64": pubkey_b64,
            "role": "user",
        })) {
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
    presented_pubkey_b64: Option<&str>,
) -> serde_json::Result<Vec<u8>> {
    let mut args = serde_json::Map::new();
    args.insert(
        "agent_ura".to_string(),
        serde_json::Value::String(user_ura.to_string()),
    );
    if let Some(public_key) = presented_pubkey_b64
        .map(str::trim)
        .filter(|public_key| !public_key.is_empty())
    {
        args.insert(
            "presented_pubkey_b64".to_string(),
            serde_json::Value::String(public_key.to_string()),
        );
    }
    serde_json::to_vec(&serde_json::Value::Object(args))
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

async fn publish_paired_user_keys_prelude(
    client: &mut InvocationClient<Channel>,
    signer: &dyn CanonicalSigner,
    user_ura: &str,
    public_keys_b64: &[String],
) -> Result<(), UserTrustBootstrapError> {
    for public_key_b64 in public_keys_b64 {
        let args = RegisterPubkeyRequest::new(user_ura, public_key_b64, TrustedAgentRole::User)
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

#[cfg(test)]
mod tests {
    use super::{
        apply_federation_join_receipt, paired_user_resolve_key_args, paired_user_trust_present,
        resolve_hosted_agent_user_segment, resolved_public_keys, sync_paired_user_trust_prelude,
        HostedAgentPreludePublicationPlan, UserTrustBootstrapError, UserTrustBootstrapOutcome,
        UserTrustSync,
    };
    use crate::daemon::ability::descriptors::{AbilityDescriptor, AdmissionAction, Visibility};
    use crate::daemon::federation::client::ability_contract::AuthorityAbilityEntry;
    use crate::daemon::federation::read_model::authority_published_abilities::AuthorityPublishedAbilityStore;
    use crate::daemon::identity::self_identity::TestCanonicalSigner;
    use crate::daemon::persistence::config::{save_credentials, state_dir, Credentials};
    use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};
    use crate::daemon::trust::cell::SharedTrustAnchor;
    use axon_sdk::pb::axon::v1::invocation_client::InvocationClient;
    use std::sync::Arc;
    use tonic::transport::Channel;

    fn canonical_authority_entry(name: &str) -> AuthorityAbilityEntry {
        AuthorityAbilityEntry {
            name: name.to_string(),
            descriptor: serde_json::to_value(
                AbilityDescriptor::new(
                    name,
                    &crate::core::ura::hub_ura("realm"),
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

    fn user_trust_sync_with_key(user_ura: &str) -> UserTrustSync {
        UserTrustSync {
            daemon_realm: "realm".to_string(),
            trust_anchor_path: std::path::PathBuf::from("/tmp/easynet-test-realm-trust.toml"),
            cell: SharedTrustAnchor::new(Arc::new(
                RealmTrustAnchor::from_entries(vec![TrustedAgent {
                    agent_ura: user_ura.to_string(),
                    public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
                    role: TrustedAgentRole::User,
                    added_at_unix_ms: 1_700_000_000_000,
                    origin_realm: None,
                    hub_endpoint: None,
                    tls_ca_pem_path: None,
                }])
                .expect("user anchor"),
            )),
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
    fn hosted_agent_owner_segment_accepts_explicit_dev_override() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        std::env::set_var("EASYNET_PAGES_USER", " dev ");

        let user_segment =
            resolve_hosted_agent_user_segment("https://hub:50443").expect("env user segment");

        assert_eq!(user_segment, "dev");
    }

    #[test]
    fn hosted_agent_owner_segment_reads_valid_paired_credentials() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        save_credentials(&paired_credentials(Some("dev"))).expect("save credentials");

        let user_segment = resolve_hosted_agent_user_segment("https://hub:50443")
            .expect("credential user segment");

        assert_eq!(user_segment, "dev");
    }

    #[test]
    fn hosted_agent_owner_segment_rejects_federation_native_credentials_without_username() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        save_credentials(&federation_native_credentials_without_user_binding())
            .expect("save federation-native device credential");

        let error = resolve_hosted_agent_user_segment("https://hub:50443")
            .expect_err("missing username must fail hosted-agent prelude");

        match error {
            crate::daemon::invocation::bidi::session_initiator::SessionError::HostedAgentPreludeFailed {
                reason,
                ..
            } => {
                assert!(
                    reason.contains("project username for hosted-agent owner projection"),
                    "{reason}"
                );
                assert!(reason.contains("no user-root Pages identity is bound"), "{reason}");
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
        let signer = TestCanonicalSigner::new("easynet:///r/realm/device/n1", [0x11; 32]);

        let outcome = sync_paired_user_trust_prelude(&mut client, &signer, &sync)
            .await
            .expect("missing credentials are the only not-required local state");
        assert_eq!(outcome, UserTrustBootstrapOutcome::NotRequired);
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
        let signer = TestCanonicalSigner::new("easynet:///r/realm/device/n1", [0x11; 32]);

        let err = sync_paired_user_trust_prelude(&mut client, &signer, &sync)
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

    #[test]
    fn paired_user_resolve_key_args_carries_presented_pubkey() {
        let body = paired_user_resolve_key_args(
            "easynet:///r/realm/user/user-dev",
            Some(" AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA= "),
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
    fn hosted_agent_prelude_plan_uses_retired_owner_cursor_generation() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let owner = "easynet:///r/realm/agent/dev.anthropic";
        let host = "easynet:///r/realm/device/n1";
        let descriptors = vec![AbilityDescriptor::new(
            "chat",
            owner,
            Visibility::Scoped,
            AdmissionAction::Invoke,
        )
        .expect("hosted-agent descriptor")];

        let first = crate::daemon::federation::read_model::owner_projection::prepare_and_persist(
            owner,
            host,
            &descriptors,
        )
        .expect("first owner projection");
        let tombstone =
            crate::daemon::federation::read_model::owner_projection::prepare_removal_and_persist(
                owner, host,
            )
            .expect("retire owner projection")
            .expect("active cursor produces tombstone");

        let plan =
            HostedAgentPreludePublicationPlan::prepare(owner, host, Some("n1"), &descriptors)
                .expect("recreated hosted-agent plan");

        assert_eq!(first.generation, 1);
        assert_eq!(tombstone.generation, first.generation);
        assert!(
            plan.generation() > tombstone.generation,
            "same-URA hosted-agent prelude must publish the recreated incarnation generation"
        );
    }
}
