use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use easynet_axon::pb::axon::v1::invocation_client::InvocationClient;
use tonic::transport::Channel;

use super::heartbeat::spawn_federation_heartbeat;
use super::supervisor::{DeviceSessionPhase, PreludeStep, SessionPhaseTracker};
use super::tasks::AbortOnDrop;
use super::SessionError;
use crate::runtime::ability_descriptor::AbilityDescriptor;
use crate::services::hub_published_ability_store::HubPublishedAbilityStore;

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
        &hub_published_abilities,
    )
    .await;
    run_owner_projection_prelude(client, phase, hub_endpoint, caller_ura, ability_descriptors)
        .await?;

    let user_trust_resync = spawn_user_trust_resync(
        client,
        channels.user_trust_resync,
        caller_ura,
        user_trust_sync,
    )
    .await;
    let federation_heartbeat = spawn_federation_heartbeat(
        channels.federation_heartbeat,
        caller_ura.to_string(),
        Arc::clone(&hub_published_abilities),
    );

    run_hosted_agent_advertise_prelude(client, phase, caller_ura).await;

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
    match send_federation_join_prelude(client, caller_ura, hub_published_abilities).await {
        Ok(()) => {
            crate::op_event!(
                component = session,
                kind = federation_join_prelude_ok,
                message = "proceeding to <self>.session",
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
                message = "proceeding to <self>.session — bidi will surface the error if join was required",
            );
        }
    }
}

async fn run_owner_projection_prelude(
    client: &mut InvocationClient<Channel>,
    phase: &mut SessionPhaseTracker,
    hub_endpoint: &str,
    caller_ura: &str,
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

async fn spawn_user_trust_resync(
    client: &mut InvocationClient<Channel>,
    resync_channel: Channel,
    caller_ura: &str,
    user_trust_sync: Option<&UserTrustSync>,
) -> Option<AbortOnDrop> {
    let sync = user_trust_sync?;
    sync_paired_user_trust_prelude(client, caller_ura, sync).await;
    let sync = sync.clone();
    let resync_caller = caller_ura.to_string();
    Some(AbortOnDrop(tokio::spawn(async move {
        let mut resync_client = InvocationClient::new(resync_channel);
        loop {
            tokio::time::sleep(USER_TRUST_RESYNC_INTERVAL).await;
            sync_paired_user_trust_prelude(&mut resync_client, &resync_caller, &sync).await;
        }
    })))
}

async fn run_hosted_agent_advertise_prelude(
    client: &mut InvocationClient<Channel>,
    phase: &mut SessionPhaseTracker,
    caller_ura: &str,
) {
    let realm = crate::ura::parse_ura(caller_ura)
        .map(|parsed| parsed.realm)
        .unwrap_or_default();
    let user_segment = std::env::var("EASYNET_PAGES_USER")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| {
            crate::persistence::config::load_credentials()
                .ok()
                .and_then(|c| c.username)
                .filter(|v| !v.is_empty())
        })
        .unwrap_or_default();

    let entries = collect_advertise_entries(&realm, &user_segment);
    if realm.is_empty() || entries.is_empty() {
        return;
    }

    let caller_node_id = crate::ura::parse_ura(caller_ura)
        .ok()
        .filter(|p| p.kind == crate::ura::URAKind::Device)
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

    let agent_registry = crate::registry::agents::load_agents().unwrap_or_default();
    let live_registry = crate::runtime::agents::build_registry();
    for entry in &entries {
        advertise_hosted_agent_entry(
            client,
            caller_ura,
            &caller_node_id,
            &agent_registry,
            &live_registry,
            entry,
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

    let local_agents_file = crate::persistence::local_agents::load().unwrap_or_default();
    for hosted in &local_agents_file.hosted_agents {
        if hosted.agent_ura.is_empty() || hosted.agent_ura.contains("<unjoined>") {
            continue;
        }
        if !seen.insert(hosted.agent_ura.clone()) {
            continue;
        }
        let short_label = crate::ura::parse_ura(&hosted.agent_ura)
            .ok()
            .filter(|p| p.kind == crate::ura::URAKind::Agent)
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
            let ura = crate::ura::agent_ura(realm, user_segment, synthetic);
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
    agent_registry: &crate::registry::agents::AgentRegistry,
    live_registry: &crate::runtime::ability_dispatch::AxonAbilityCatalog,
    entry: &AdvertiseEntry,
) {
    let agent_id = crate::ura::parse_ura(&entry.agent_ura)
        .ok()
        .filter(|p| p.kind == crate::ura::URAKind::Agent)
        .and_then(|p| p.agent_ids().map(|(_, agent_id)| agent_id.to_string()))
        .unwrap_or_default();
    let host_for_advertise = if is_user_scoped_synthetic_agent(&agent_id) {
        None
    } else {
        caller_node_id.as_deref()
    };
    let advertise_agent_result =
        send_advertise_agent_prelude(client, caller_ura, &entry.agent_ura, host_for_advertise)
            .await;
    match advertise_agent_result {
        Ok(()) => {
            advertise_hosted_agent_abilities(
                client,
                caller_ura,
                caller_node_id,
                agent_registry,
                live_registry,
                entry,
                &agent_id,
            )
            .await;
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

async fn advertise_hosted_agent_abilities(
    client: &mut InvocationClient<Channel>,
    caller_ura: &str,
    caller_node_id: &Option<String>,
    agent_registry: &crate::registry::agents::AgentRegistry,
    live_registry: &crate::runtime::ability_dispatch::AxonAbilityCatalog,
    entry: &AdvertiseEntry,
    agent_id: &str,
) {
    let descriptors = match entry.hosted_agent_name.as_deref() {
        Some(agent_name) => {
            let Some(agent_config) = agent_registry.agents.get(agent_name) else {
                return;
            };
            build_hosted_agent_ability_descriptors(
                &entry.agent_ura,
                agent_name,
                agent_config,
                caller_node_id.as_deref(),
                live_registry,
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
        client,
        caller_ura,
        &entry.agent_ura,
        caller_ura,
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
    hub_published_abilities: &HubPublishedAbilityStore,
) -> Result<(), tonic::Status> {
    let realm = crate::ura::parse_ura(caller_ura)
        .map(|parsed| parsed.realm)
        .unwrap_or_default();

    let body = serde_json::json!({
        "membership_ura": caller_ura,
        "realm": realm,
    });
    let arguments = serde_json::to_vec(&body)
        .map_err(|e| tonic::Status::internal(format!("federation.join prelude serialize: {e}")))?;

    let request = crate::services::invocation_transport::ProtoEnvelope::caller_only(caller_ura)
        .and_then(|env| env.invoke_request("federation.join", arguments))
        .map_err(|e| tonic::Status::invalid_argument(format!("federation.join prelude: {e}")))?;

    match client.invoke(request).await {
        Ok(reply) => {
            let body_bytes = reply.into_inner().result;
            if !body_bytes.is_empty() {
                if let Ok(body) = serde_json::from_slice::<
                    crate::runtime::federation_client::JoinReceipt,
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

    let request = crate::services::invocation_transport::ProtoEnvelope::caller_only(caller_ura)
        .and_then(|env| env.invoke_request("federation.advertise_agent", arguments))
        .map_err(|e| {
            tonic::Status::invalid_argument(format!("federation.advertise_agent prelude: {e}"))
        })?;

    invoke_prelude_unary(client, request, "federation.advertise_agent")
        .await
        .map(|_| ())
}

async fn send_advertise_abilities_prelude(
    client: &mut InvocationClient<Channel>,
    caller_ura: &str,
    owner_ura: &str,
    host_device_ura: &str,
    descriptors: &[AbilityDescriptor],
) -> Result<(), tonic::Status> {
    let projection = crate::runtime::owner_projection::prepare_and_persist(
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

    let request = crate::services::invocation_transport::ProtoEnvelope::caller_only(caller_ura)
        .and_then(|env| env.invoke_request("federation.advertise_abilities", arguments))
        .map_err(|e| {
            tonic::Status::invalid_argument(format!("federation.advertise_abilities prelude: {e}"))
        })?;

    invoke_prelude_unary(client, request, "federation.advertise_abilities")
        .await
        .map(|_| ())
}

#[derive(Clone)]
pub struct UserTrustSync {
    pub daemon_realm: String,
    pub trust_anchor_path: PathBuf,
    pub cell: crate::services::trust_anchor_cell::SharedTrustAnchor,
}

const USER_TRUST_RESYNC_INTERVAL: Duration = Duration::from_secs(60);

async fn sync_paired_user_trust_prelude(
    client: &mut InvocationClient<Channel>,
    caller_ura: &str,
    sync: &UserTrustSync,
) {
    let Ok(creds) = crate::persistence::config::load_credentials() else {
        return;
    };
    let Some(username) = creds
        .username
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return;
    };
    let realm = creds.realm.trim();
    if realm != sync.daemon_realm {
        return;
    }
    let user_ura = crate::ura::user_ura(realm, username);

    let args = match serde_json::to_vec(&serde_json::json!({ "agent_ura": user_ura })) {
        Ok(v) => v,
        Err(_) => return,
    };
    let request = match crate::services::invocation_transport::ProtoEnvelope::caller_only(
        caller_ura,
    )
    .and_then(|env| {
        env.invoke_request(
            crate::services::invocation_transport::federation_wrappers::ABILITY_FEDERATION_RESOLVE_KEY,
            args,
        )
    }) {
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
                kind = user_trust_sync_resolve_failed,
                code = code,
                error = msg,
                user_ura = user_ura,
            );
            return;
        }
    };

    let parsed = serde_json::from_slice::<serde_json::Value>(&response.result).ok();
    let mut pubkeys: Vec<String> = parsed
        .as_ref()
        .and_then(|v| v.get("public_keys_b64"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|k| k.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if pubkeys.is_empty() {
        if let Some(pk) = parsed
            .as_ref()
            .and_then(|v| v.get("public_key_b64"))
            .and_then(|pk| pk.as_str())
        {
            pubkeys.push(pk.to_string());
        }
    }
    if pubkeys.is_empty() {
        crate::op_event!(
            component = session,
            kind = user_trust_sync_resolve_empty,
            user_ura = user_ura,
            message = "hub returned no user keys — user key not registered at hub yet",
        );
        return;
    }

    for pubkey_b64 in pubkeys {
        let register_args = match serde_json::to_vec(&serde_json::json!({
            "agent_ura": user_ura,
            "public_key_b64": pubkey_b64,
            "role": "user",
        })) {
            Ok(v) => v,
            Err(_) => continue,
        };
        match crate::services::invocation_transport::register_device_pubkey::handle(
            &register_args,
            &sync.daemon_realm,
            &sync.trust_anchor_path,
            &sync.cell,
        ) {
            Ok(_) => {
                crate::op_event!(
                    component = session,
                    kind = user_trust_sync_ok,
                    user_ura = user_ura,
                );
            }
            Err(status) if status.code() == tonic::Code::AlreadyExists => {
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
            }
        }
    }
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
    entry: &crate::registry::agents::AgentEntry,
    host_node_id: Option<&str>,
    live_registry: &crate::runtime::ability_dispatch::AxonAbilityCatalog,
) -> Vec<AbilityDescriptor> {
    let mut descriptors = Vec::new();
    for spec in crate::runtime::abilities::abilities_for_publication(agent_name, entry) {
        let registry_name = spec.name();
        let owner_local_name = crate::runtime::abilities::public_agent_ability_name(
            owner_ura,
            agent_name,
            registry_name,
        );
        let Ok(mut descriptor) = AbilityDescriptor::new(
            owner_local_name,
            owner_ura,
            crate::runtime::ability_descriptor::Visibility::Scoped,
        ) else {
            continue;
        };
        descriptor = descriptor
            .with_description(spec.description())
            .with_input_schema(spec.parameters().clone())
            .with_hints(crate::runtime::agents::discovery_hints_for(
                live_registry,
                registry_name,
            ))
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

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn build_synthetic_pages_ability_descriptors(owner_ura: &str) -> Vec<AbilityDescriptor> {
    crate::runtime::agents::pages::management_ability_specs()
        .into_iter()
        .filter_map(|spec| {
            let descriptor_name = format!("pages.{}", spec.relative_name);
            AbilityDescriptor::new(
                descriptor_name,
                owner_ura,
                crate::runtime::ability_descriptor::Visibility::Scoped,
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
