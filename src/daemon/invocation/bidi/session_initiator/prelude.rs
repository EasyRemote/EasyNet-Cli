use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use easynet_axon::pb::axon::v1::{invocation_client::InvocationClient, InvokeRequest};
use tonic::{transport::Channel, Status};

use super::envelope::SessionSigningSeed;
use super::heartbeat::spawn_federation_heartbeat;
use super::supervisor::{DeviceSessionPhase, PreludeStep, SessionPhaseTracker};
use super::tasks::AbortOnDrop;
use super::SessionError;
use crate::daemon::ability::descriptors::AbilityDescriptor;
use crate::daemon::federation::read_model::hub_published_abilities::HubPublishedAbilityStore;

pub struct SessionPreludeInputs<'a> {
    pub(super) ability_descriptors: &'a [AbilityDescriptor],
    pub(super) hub_published_abilities: Arc<HubPublishedAbilityStore>,
}

impl<'a> SessionPreludeInputs<'a> {
    #[must_use]
    pub fn new(
        ability_descriptors: &'a [AbilityDescriptor],
        hub_published_abilities: Arc<HubPublishedAbilityStore>,
    ) -> Self {
        Self {
            ability_descriptors,
            hub_published_abilities,
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
    pub(super) caller_ura: &'a str,
    pub(super) signing_seed: Option<SessionSigningSeed>,
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
        caller_ura,
        signing_seed,
        inputs,
        user_trust_sync,
        channels,
    } = request;
    let ability_descriptors = inputs.ability_descriptors;
    let hub_published_abilities = inputs.hub_published_abilities;

    run_join_prelude(
        client,
        phase,
        hub_endpoint,
        caller_ura,
        signing_seed,
        &hub_published_abilities,
    )
    .await;
    run_owner_projection_prelude(
        client,
        phase,
        hub_endpoint,
        caller_ura,
        signing_seed,
        ability_descriptors,
    )
    .await?;

    let user_trust_resync = run_user_trust_bootstrap_and_spawn_resync(
        client,
        phase,
        channels.user_trust_resync,
        caller_ura,
        signing_seed,
        user_trust_sync,
    )
    .await
    .map_err(|source| SessionError::UserTrustBootstrapFailed {
        endpoint: hub_endpoint.to_string(),
        source,
    })?;
    let federation_heartbeat = spawn_federation_heartbeat(
        channels.federation_heartbeat,
        caller_ura.to_string(),
        signing_seed,
        Arc::clone(&hub_published_abilities),
    );

    run_hosted_agent_advertise_prelude(client, phase, caller_ura, signing_seed).await;

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
    signing_seed: Option<SessionSigningSeed>,
    hub_published_abilities: &HubPublishedAbilityStore,
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
    match send_federation_join_prelude(client, caller_ura, signing_seed, hub_published_abilities)
        .await
    {
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
    signing_seed: Option<SessionSigningSeed>,
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
        caller_ura,
        signing_seed,
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
    caller_ura: &str,
    signing_seed: Option<SessionSigningSeed>,
    user_trust_sync: Option<&UserTrustSync>,
) -> Result<Option<AbortOnDrop>, UserTrustBootstrapError> {
    let Some(sync) = user_trust_sync else {
        return Ok(None);
    };
    phase.transition(
        DeviceSessionPhase::Preluding(PreludeStep::TrustBootstrap),
        "owner_projection_published",
    );
    sync_realm_hub_trust_prelude(client, caller_ura, signing_seed, sync).await;
    let outcome = sync_paired_user_trust_prelude(client, caller_ura, signing_seed, sync).await?;
    log_user_trust_bootstrap_outcome(&outcome);
    let sync = sync.clone();
    let resync_caller = caller_ura.to_string();
    Ok(Some(AbortOnDrop(tokio::spawn(async move {
        let mut resync_client = InvocationClient::new(resync_channel);
        loop {
            tokio::time::sleep(USER_TRUST_RESYNC_INTERVAL).await;
            sync_realm_hub_trust_prelude(&mut resync_client, &resync_caller, signing_seed, &sync)
                .await;
            if let Err(err) = sync_paired_user_trust_prelude(
                &mut resync_client,
                &resync_caller,
                signing_seed,
                &sync,
            )
            .await
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
    caller_ura: &str,
    signing_seed: Option<SessionSigningSeed>,
) {
    let realm = crate::core::ura::parse_ura(caller_ura)
        .map(|parsed| parsed.realm)
        .unwrap_or_default();
    // The agent owner-prefix is the USERNAME slug (`<username>.<agent>`, e.g.
    // `dev.pages`), NOT the user UUID. This is the §15.1-3 dual grammar: subject
    // URAs anchor on the stable UUID, but owner-prefixed agent/resource URAs keep
    // the username slug. The backend resolves these owners via
    // `svc.UsernameForUID` (username), so advertising under the UUID
    // (`<uuid>.pages`) lands a directory entry the resolver never queries →
    // `namespace.resolve NXDOMAIN: owner is not online` on `pages.list`/etc.
    let user_segment = std::env::var("EASYNET_PAGES_USER")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| {
            crate::daemon::persistence::config::load_credentials()
                .ok()
                .and_then(|c| c.username)
                .filter(|v| !v.is_empty())
        })
        .unwrap_or_default();

    let entries = collect_advertise_entries(&realm, &user_segment);
    if realm.is_empty() || entries.is_empty() {
        return;
    }

    let caller_node_id = crate::core::ura::parse_ura(caller_ura)
        .ok()
        .filter(|p| p.kind == crate::core::ura::URAKind::Device)
        .and_then(|p| p.device_id().map(str::to_string));
    let entries_count = entries.len();
    let labels_display = format!(
        "{:?}",
        entries.iter().map(|e| &e.short_label).collect::<Vec<_>>()
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

    let agent_registry =
        crate::daemon::persistence::agent_registry::load_agents().unwrap_or_default();
    let live_registry = crate::daemon::ability::catalog::build_registry();
    for entry in &entries {
        advertise_hosted_agent_entry(
            client,
            caller_ura,
            &caller_node_id,
            &agent_registry,
            &live_registry,
            entry,
            signing_seed,
        )
        .await;
    }
    let entries_done_count = entries.len();
    crate::op_event!(
        component = session,
        kind = advertise_agent_prelude_done,
        agent_count = entries_done_count,
    );
}

#[derive(Debug, Clone)]
struct AdvertiseEntry {
    agent_ura: String,
    short_label: String,
    hosted_agent_name: Option<String>,
}

fn collect_advertise_entries(realm: &str, user_segment: &str) -> Vec<AdvertiseEntry> {
    let mut entries = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    let local_agents_file = crate::daemon::persistence::local_agents::load().unwrap_or_default();
    for hosted in &local_agents_file.hosted_agents {
        if hosted.agent_ura.is_empty() || hosted.agent_ura.contains("<unjoined>") {
            continue;
        }
        if !seen.insert(hosted.agent_ura.clone()) {
            continue;
        }
        let short_label = crate::core::ura::parse_ura(&hosted.agent_ura)
            .ok()
            .filter(|p| p.kind == crate::core::ura::URAKind::Agent)
            .and_then(|p| {
                p.agent_ids()
                    .map(|(user_id, agent_id)| format!("{user_id}.{agent_id}"))
            })
            .unwrap_or_else(|| hosted.agent_ura.clone());
        entries.push(AdvertiseEntry {
            agent_ura: hosted.agent_ura.clone(),
            short_label,
            hosted_agent_name: (hosted.profile == "llm").then(|| hosted.name.clone()),
        });
    }

    if !realm.is_empty() && !user_segment.is_empty() && user_segment != "self" {
        for synthetic in ["pages", "files"] {
            let ura = crate::core::ura::agent_ura(realm, user_segment, synthetic);
            if seen.insert(ura.clone()) {
                entries.push(AdvertiseEntry {
                    agent_ura: ura,
                    short_label: format!("{user_segment}.{synthetic}"),
                    hosted_agent_name: None,
                });
            }
        }
    }

    entries
}

async fn advertise_hosted_agent_entry(
    client: &mut InvocationClient<Channel>,
    caller_ura: &str,
    caller_node_id: &Option<String>,
    agent_registry: &crate::daemon::persistence::agent_registry::AgentRegistry,
    live_registry: &crate::daemon::ability::dispatch::AxonAbilityCatalog,
    entry: &AdvertiseEntry,
    signing_seed: Option<SessionSigningSeed>,
) {
    let agent_id = crate::core::ura::parse_ura(&entry.agent_ura)
        .ok()
        .filter(|p| p.kind == crate::core::ura::URAKind::Agent)
        .and_then(|p| p.agent_ids().map(|(_, agent_id)| agent_id.to_string()))
        .unwrap_or_default();
    let host_for_advertise = if is_user_scoped_synthetic_agent(&agent_id) {
        None
    } else {
        caller_node_id.as_deref()
    };
    let advertise_agent_result = send_advertise_agent_prelude(
        client,
        caller_ura,
        &entry.agent_ura,
        host_for_advertise,
        signing_seed,
    )
    .await;
    match advertise_agent_result {
        Ok(()) => {
            let mut advertise_ctx = HostedAgentAbilityAdvertiseContext {
                client,
                caller_ura,
                caller_node_id: caller_node_id.as_deref(),
                agent_registry,
                live_registry,
                signing_seed,
            };
            advertise_hosted_agent_abilities(&mut advertise_ctx, entry, &agent_id).await;
        }
        Err(err) => {
            let agent_ura = entry.agent_ura.clone();
            let code = err.code();
            let msg = err.message();
            crate::op_event!(
                component = session,
                kind = advertise_agent_prelude_soft_failed,
                agent_ura = agent_ura,
                code = code,
                error = msg,
            );
        }
    }
}

fn is_user_scoped_synthetic_agent(agent_id: &str) -> bool {
    matches!(agent_id, "pages" | "files")
}

struct HostedAgentAbilityAdvertiseContext<'a> {
    client: &'a mut InvocationClient<Channel>,
    caller_ura: &'a str,
    caller_node_id: Option<&'a str>,
    agent_registry: &'a crate::daemon::persistence::agent_registry::AgentRegistry,
    live_registry: &'a crate::daemon::ability::dispatch::AxonAbilityCatalog,
    signing_seed: Option<SessionSigningSeed>,
}

async fn advertise_hosted_agent_abilities(
    ctx: &mut HostedAgentAbilityAdvertiseContext<'_>,
    entry: &AdvertiseEntry,
    agent_id: &str,
) {
    let descriptors = match entry.hosted_agent_name.as_deref() {
        Some(agent_name) => {
            let Some(agent_config) = ctx.agent_registry.agents.get(agent_name) else {
                return;
            };
            build_hosted_agent_ability_descriptors(
                &entry.agent_ura,
                agent_name,
                agent_config,
                ctx.caller_node_id,
                ctx.live_registry,
            )
        }
        None if agent_id == "pages" => build_synthetic_pages_ability_descriptors(&entry.agent_ura),
        None => return,
    };
    if descriptors.is_empty() {
        return;
    }

    let ability_count = descriptors.len();
    crate::op_event!(
        component = session,
        kind = advertise_hosted_agent_abilities_prelude_sending,
        agent_ura = entry.agent_ura,
        ability_count = ability_count,
    );
    if let Err(err) = send_advertise_abilities_prelude(
        ctx.client,
        ctx.caller_ura,
        &entry.agent_ura,
        ctx.caller_ura,
        ctx.signing_seed,
        &descriptors,
    )
    .await
    {
        let code = err.code();
        let msg = err.message();
        crate::op_event!(
            component = session,
            kind = advertise_hosted_agent_abilities_prelude_soft_failed,
            agent_ura = entry.agent_ura,
            code = code,
            error = msg,
        );
    } else {
        crate::op_event!(
            component = session,
            kind = advertise_hosted_agent_abilities_prelude_ok,
            agent_ura = entry.agent_ura,
            ability_count = ability_count,
        );
    }
}

async fn send_federation_join_prelude(
    client: &mut InvocationClient<Channel>,
    caller_ura: &str,
    signing_seed: Option<SessionSigningSeed>,
    hub_published_abilities: &HubPublishedAbilityStore,
) -> Result<(), tonic::Status> {
    let realm = crate::core::ura::parse_ura(caller_ura)
        .map(|parsed| parsed.realm)
        .unwrap_or_default();

    let body = serde_json::json!({
        "membership_ura": caller_ura,
        "realm": realm,
    });
    let arguments = serde_json::to_vec(&body)
        .map_err(|e| tonic::Status::internal(format!("federation.join prelude serialize: {e}")))?;

    let request = signed_prelude_request(
        caller_ura,
        caller_ura,
        "federation.join",
        arguments,
        signing_seed,
    )?;

    match client.invoke(request).await {
        Ok(reply) => {
            let body_bytes = reply.into_inner().result;
            if !body_bytes.is_empty() {
                if let Ok(body) = serde_json::from_slice::<
                    crate::daemon::federation::client::ability_contract::JoinReceipt,
                >(&body_bytes)
                {
                    hub_published_abilities.seed_from_snapshot(
                        body.hub_abilities_revision,
                        body.hub_published_abilities,
                    );
                    if !hub_published_abilities.is_empty() {
                        let ability_count = hub_published_abilities.len();
                        let hub_abilities_revision = body.hub_abilities_revision;
                        crate::op_event!(
                            component = session,
                            kind = hub_broadcast_abilities_seeded,
                            ability_count = ability_count,
                            hub_abilities_revision = hub_abilities_revision,
                        );
                    }
                }
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

async fn send_advertise_agent_prelude(
    client: &mut InvocationClient<Channel>,
    caller_ura: &str,
    agent_ura: &str,
    host_node_id: Option<&str>,
    signing_seed: Option<SessionSigningSeed>,
) -> Result<(), tonic::Status> {
    let mut body = serde_json::json!({
        "agent_ura": agent_ura,
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

    let request = signed_prelude_request(
        caller_ura,
        agent_ura,
        "federation.advertise_agent",
        arguments,
        signing_seed,
    )?;

    invoke_prelude_unary(client, request, "federation.advertise_agent")
        .await
        .map(|_| ())
}

async fn send_advertise_abilities_prelude(
    client: &mut InvocationClient<Channel>,
    caller_ura: &str,
    owner_ura: &str,
    host_device_ura: &str,
    signing_seed: Option<SessionSigningSeed>,
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

    let body = serde_json::json!({
        "owner_ura": projection.owner_ura,
        "host_device_ura": projection.host_device_ura,
        "projection_revision": projection.projection_revision,
        "projection_digest": projection.projection_digest,
        "lease_expires_unix_ms": projection.lease_expires_unix_ms,
        "ability_summaries": projection.ability_summaries,
    });
    let arguments = serde_json::to_vec(&body).map_err(|e| {
        tonic::Status::internal(format!(
            "federation.advertise_abilities prelude serialize: {e}"
        ))
    })?;

    let request = signed_prelude_request(
        caller_ura,
        owner_ura,
        "federation.advertise_abilities",
        arguments,
        signing_seed,
    )?;

    invoke_prelude_unary(client, request, "federation.advertise_abilities")
        .await
        .map(|_| ())
}

pub(super) fn signed_prelude_request(
    caller_ura: &str,
    subject_ura: &str,
    function_name: &str,
    arguments: Vec<u8>,
    signing_seed: Option<SessionSigningSeed>,
) -> Result<InvokeRequest, Status> {
    let hub_ura = session_hub_ura(caller_ura)?;
    let descriptor_subject_ura =
        descriptor_prelude_subject_ura(&hub_ura, subject_ura, function_name)?;
    let mut request = crate::daemon::invocation::ProtoEnvelope::targeted(
        caller_ura,
        hub_ura,
        descriptor_subject_ura,
    )
    .and_then(|env| env.invoke_request(function_name, arguments))
    .map_err(|e| Status::invalid_argument(format!("{function_name} prelude: {e}")))?;
    if let Some(seed) = signing_seed {
        sign_descriptor_bound_prelude_request(&mut request, function_name, &seed)?;
    }
    Ok(request)
}

fn sign_descriptor_bound_prelude_request(
    request: &mut InvokeRequest,
    function_name: &str,
    seed: &SessionSigningSeed,
) -> Result<(), Status> {
    use ed25519_dalek::{Signer as _, SigningKey};

    let envelope = request.envelope.as_mut().ok_or_else(|| {
        Status::internal(format!("{function_name} prelude request missing envelope"))
    })?;
    let caller_ura = envelope
        .caller
        .as_ref()
        .map(|caller| caller.ura.trim().to_string())
        .filter(|ura| !ura.is_empty())
        .ok_or_else(|| Status::internal(format!("{function_name} prelude missing caller URA")))?;
    let callee_ura = envelope
        .callee
        .as_ref()
        .map(|callee| callee.ura.trim())
        .filter(|ura| !ura.is_empty())
        .ok_or_else(|| Status::internal(format!("{function_name} prelude missing callee URA")))?;
    // `function_name` is a bare ability name (`federation.advertise_abilities`),
    // NOT a `<ability-ura>@<version>` descriptor ref. Build the canonical ref
    // from (callee hub URA, ability name, default descriptor version) — the hub
    // ingress treats `federation.*` as descriptor-bound on the wire and rejects
    // a bare name with `ability_descriptor_ref_malformed`. `require_*` only
    // VALIDATES an already-formed ref; the prelude must CONSTRUCT one. (Commit
    // 22187b3f tightened ingress + removed the old resolver but left this egress
    // site passing the bare name — the egress/ingress asymmetry that wedged
    // session.open into an advertise_abilities reconnect loop.)
    let descriptor_ref =
        crate::daemon::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
            callee_ura,
            function_name,
            crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
        )
        .map_err(|err| {
            Status::internal(format!(
                "{function_name} prelude signing requires an explicit descriptor ref: {err}"
            ))
        })?;
    let descriptor_bound =
        crate::daemon::axon_bridge::wire_descriptor::descriptor_bound_from_wire_parts(
            envelope.clone(),
            descriptor_ref.clone(),
            &request.arguments,
            crate::daemon::axon_bridge::wire_descriptor::WireCallerIdentity::FromEnvelope,
        )
        .map_err(|err| {
            Status::internal(format!(
                "{function_name} prelude descriptor-bound envelope failed: {err}"
            ))
        })?;
    let signing_key = SigningKey::from_bytes(seed);
    let signature = signing_key.sign(&descriptor_bound.envelope.canonical_bytes());
    envelope.caller_signature = Some(easynet_axon::pb::axon::v1::CallerSignature {
        algorithm: "ed25519".to_string(),
        signature: signature.to_bytes().to_vec(),
        key_id_hint: caller_ura,
    });
    request.metadata.insert(
        crate::daemon::invocation::dispatch::invocation_wire::SIGNED_DESCRIPTOR_REF_METADATA_KEY
            .to_string(),
        descriptor_ref,
    );
    Ok(())
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
    AlreadyTrusted { user_ura: String },
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
        UserTrustBootstrapOutcome::AlreadyTrusted { user_ura } => {
            crate::op_event!(
                component = session,
                kind = user_trust_bootstrap_already_trusted,
                user_ura = user_ura,
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
    caller_ura: &str,
    signing_seed: Option<SessionSigningSeed>,
    sync: &UserTrustSync,
) {
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
        caller_ura,
        &hub_ura,
        crate::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_RESOLVE_KEY,
        args,
        signing_seed,
    ) {
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

    let pubkeys = resolved_public_keys(&response.result);
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
    caller_ura: &str,
    signing_seed: Option<SessionSigningSeed>,
    sync: &UserTrustSync,
) -> Result<UserTrustBootstrapOutcome, UserTrustBootstrapError> {
    let Ok(creds) = crate::daemon::persistence::config::load_credentials() else {
        return Ok(UserTrustBootstrapOutcome::NotRequired);
    };
    let Ok(user_ura) = creds.user_ura() else {
        return Ok(UserTrustBootstrapOutcome::NotRequired);
    };
    let realm = creds.realm.trim();
    if realm != sync.daemon_realm {
        return Ok(UserTrustBootstrapOutcome::NotRequired);
    }
    if paired_user_trust_present(sync, &user_ura) {
        return Ok(UserTrustBootstrapOutcome::AlreadyTrusted { user_ura });
    }

    let args = match serde_json::to_vec(&serde_json::json!({ "agent_ura": user_ura })) {
        Ok(value) => value,
        Err(err) => {
            return Err(UserTrustBootstrapError::ResolveFailed {
                user_ura,
                status: tonic::Status::internal(format!(
                    "federation.resolve_key user args encode failed: {err}"
                )),
            });
        }
    };
    let request = match signed_prelude_request(
        caller_ura,
        &user_ura,
        crate::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_RESOLVE_KEY,
        args,
        signing_seed,
    ) {
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

    let pubkeys = resolved_public_keys(&response.result);
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
            "agent_ura": user_ura,
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

fn resolved_public_keys(result: &[u8]) -> Vec<String> {
    let parsed = serde_json::from_slice::<serde_json::Value>(result).ok();
    let mut pubkeys: Vec<String> = parsed
        .as_ref()
        .and_then(|v| v.get("public_keys_b64"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|k| {
                    let key = k.as_str()?.trim();
                    (!key.is_empty()).then(|| key.to_string())
                })
                .collect()
        })
        .unwrap_or_default();
    if pubkeys.is_empty() {
        if let Some(pk) = parsed
            .as_ref()
            .and_then(|v| v.get("public_key_b64"))
            .and_then(|pk| pk.as_str())
            .map(str::trim)
            .filter(|pk| !pk.is_empty())
        {
            pubkeys.push(pk.to_string());
        }
    }
    pubkeys
}

fn paired_user_trust_present(sync: &UserTrustSync, user_ura: &str) -> bool {
    !sync.cell.snapshot().lookup_user_all(user_ura).is_empty()
}

pub(super) async fn invoke_prelude_unary(
    client: &mut InvocationClient<Channel>,
    request: easynet_axon::pb::axon::v1::InvokeRequest,
    ability_name: &str,
) -> Result<easynet_axon::pb::axon::v1::InvokeResponse, tonic::Status> {
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

fn build_hosted_agent_ability_descriptors(
    owner_ura: &str,
    agent_name: &str,
    entry: &crate::daemon::persistence::agent_registry::AgentEntry,
    host_node_id: Option<&str>,
    live_registry: &crate::daemon::ability::dispatch::AxonAbilityCatalog,
) -> Vec<AbilityDescriptor> {
    let mut descriptors = Vec::new();
    let hint_snapshot =
        crate::daemon::ability::catalog::AbilityDiscoveryHintSnapshot::from_registry(live_registry);
    for spec in crate::daemon::execution::mission::agent_ability_specs::abilities_for_publication(
        agent_name, entry,
    ) {
        let registry_name = spec.name();
        let owner_local_name =
            crate::daemon::execution::mission::agent_ability_specs::public_agent_ability_name(
                owner_ura,
                agent_name,
                registry_name,
            );
        let Ok(mut descriptor) = AbilityDescriptor::new(
            owner_local_name,
            owner_ura,
            crate::daemon::ability::descriptors::Visibility::Scoped,
        ) else {
            continue;
        };
        descriptor = descriptor
            .with_description(spec.description())
            .with_input_schema(spec.parameters().clone())
            .with_hints(hint_snapshot.for_name(registry_name))
            .with_source(format!("agent:{agent_name}"))
            .with_metadata_entry("runtime", entry.agent_type.to_string())
            .with_metadata_entry("agent_type", entry.agent_type.to_string())
            .with_metadata_entry("base_runtime", entry.agent_type.to_string());
        if let Some(node_id) = host_node_id {
            descriptor = descriptor.with_metadata_entry("host_node_id", node_id.to_string());
        }
        if let Some(model) = entry.model.as_ref() {
            descriptor = descriptor
                .with_metadata_entry("model", model.clone())
                .with_metadata_entry("base_model", model.clone());
        }
        descriptors.push(descriptor);
    }
    descriptors
}

pub(super) fn build_synthetic_pages_ability_descriptors(owner_ura: &str) -> Vec<AbilityDescriptor> {
    crate::daemon::ability::builtins::resources::pages::management_ability_specs()
        .into_iter()
        .filter_map(|spec| {
            let descriptor_name = format!("pages.{}", spec.relative_name);
            AbilityDescriptor::new(
                descriptor_name,
                owner_ura,
                crate::daemon::ability::descriptors::Visibility::Scoped,
            )
            .ok()
            .map(|descriptor| {
                descriptor
                    .with_description(spec.description)
                    .with_input_schema(spec.input_schema)
                    .with_source("synthetic:pages")
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{paired_user_trust_present, resolved_public_keys, UserTrustSync};
    use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};
    use crate::daemon::trust::cell::SharedTrustAnchor;
    use std::sync::Arc;

    #[test]
    fn resolved_public_keys_prefers_array_response() {
        let body = br#"{
            "public_key_b64": "fallback",
            "public_keys_b64": [" key-a ", "", 7, "key-b"]
        }"#;

        assert_eq!(
            resolved_public_keys(body),
            vec!["key-a".to_string(), "key-b".to_string()]
        );
    }

    #[test]
    fn resolved_public_keys_falls_back_to_single_key() {
        let body = br#"{ "public_key_b64": " single-key " }"#;

        assert_eq!(resolved_public_keys(body), vec!["single-key".to_string()]);
    }

    #[test]
    fn resolved_public_keys_ignores_malformed_or_empty_payloads() {
        assert!(resolved_public_keys(br#"{"public_keys_b64":[]}"#).is_empty());
        assert!(resolved_public_keys(br#"not-json"#).is_empty());
        assert!(resolved_public_keys(br#"{ "public_key_b64": " " }"#).is_empty());
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
}
