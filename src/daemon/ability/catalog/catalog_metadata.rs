// EasyNet CLI — published-ability catalogue metadata
// ==================================================
//
// The read-only descriptor surface: published names/metadata,
// descriptor paths, descriptions, input schemas, RFC-006 rows.
// Catalog metadata owner for daemon-owned system abilities.

use super::{build_registry, build_system_registry};
use std::collections::{BTreeMap, BTreeSet};

use crate::daemon::ability::builtins::{
    agents::{
        chat_history as chat_history_ability, discover as discover_ability,
        lifecycle as agent_lifecycle_ability, list as agent_list_ability,
    },
    automation::{
        discuss as discuss_ability, loop_ability, mission as mission_ability,
        orchestration as orchestration_ability, schedule as schedule_ability,
        think as think_ability,
    },
    device_control::{
        ability_management::{ops as device_ops_ability, publish as ability_publish_ability},
        browser as browser_session_ability, file_edit as fs_edit_ability,
        file_transfer as file_transfer_ability, files as fs_ability, http as http_request_ability,
        process as process_exec_ability, session as session_ability, shell as shell_run_ability,
        terminal::{
            attach as pty_attach_ability, io as pty_io_ability, lifecycle as pty_lifecycle_ability,
        },
    },
    governance::{
        admin_status as admin_status_ability, consent as permission_ability, health as ping,
        invocation_history as invocation_history_ability, meta as meta_ability,
        network_health as network_health_ability, teach as teach_ability,
    },
    integrations::{
        a2a::{bridge as a2a_bridge_ability, client as a2a_client_ability},
        mcp::{bridge as mcp_bridge_ability, client as mcp_client_ability},
        plugins as plugin_lifecycle_ability,
    },
    resources::{
        context::ability as context_ability,
        list as list_resources_ability, media,
        skills::{install as skill_install_ability, publish as skill_publish_ability},
        voice as voice_call_ability,
    },
};
use crate::daemon::ability::catalog::{ability_toml, system_ability_descriptor_path};
use crate::daemon::ability::descriptors::AbilityHints;
use crate::daemon::ability::dispatch::AxonAbilityCatalog;
use crate::daemon::ability::names::{
    agents as agent_names, automation as automation_names, device_control as device_names,
    federation as federation_names, governance as governance_names,
    integrations as integration_names, resources as resource_names,
};
use crate::daemon::ability::CallMode as DescriptorCallMode;

/// Public list of every v1 system-ability *name*. Used by
/// `registry::a2a_labels` to populate the top-level
/// `system_skills[]` field of the node-roster v2 envelope so peers
/// discover what device-profile abilities this daemon offers without invoking
/// anything.
///
/// The list is built from the live registry to avoid name drift
/// between the publisher and the runtime catalogue.
///
/// RFC-005 public catalogue names are owner-local names. Device-profile-owned
/// handlers may still use implementation-local registry keys while routing,
/// but public discovery must expose `fs.read`, `skill.list`, `agent.list`,
/// etc.; the owner is carried by `owner_ura` / `ability_ura`, not duplicated
/// in the ability name.
pub fn published_ability_names() -> Vec<String> {
    build_registry()
        .list_abilities()
        .into_iter()
        .filter(|name| is_publishable_catalog_name(name))
        .collect()
}

/// Public catalogue filter after the RFC-005 cleanup.
///
/// No legacy dual-registration remains. Keep this as a named predicate because
/// the two catalogue builders share the same surface and because future
/// non-publishable synthetic rows should be excluded here, not by ad-hoc
/// prefix checks in callers.
pub fn is_publishable_catalog_name(name: &str) -> bool {
    // Local front door only. The daemon registers this key so the CLI can
    // call aggregate discovery without picking an arbitrary self agent, but
    // it is not a public/federated capability. Publishing it would duplicate
    // the device owner prefix and break RFC-005 owner-local names.
    name != discover_ability::DEVICE_DISCOVER_ABILITY
}

/// One row of a system ability's discovery + registration metadata.
///
/// Centralises (name, description, input_schema) so every consumer —
/// the federation label builder (`registry::a2a_labels`), the
/// runtime-local register publisher (`daemon::federation::publish`), and any
/// future `easynet ability list --system` surface — pulls from one
/// table. Adding a new system ability now requires updating exactly
/// one match arm in `metadata_for`; previously the same name lived
/// in three places that could (and did) drift.
#[derive(Debug, Clone)]
pub struct SystemAbilityMetadata {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub hints: crate::daemon::ability::descriptors::AbilityHints,
}

/// Every published system ability's metadata, in the deterministic
/// order `published_ability_names()` returns.
///
/// `<agent>.chat` is **excluded** even when the live registry would
/// include it: those entries are already published to the
/// axon-runtime via `daemon::federation::publish::republish_abilities_via_advertise`
/// off the on-disk `chat.ability.toml` manifest, and re-publishing
/// them through the system path would double-register with a
/// different (synthesised) schema. Filter is by suffix because the
/// agent name varies per install.
pub fn published_abilities() -> Vec<SystemAbilityMetadata> {
    let registry = build_registry();
    published_abilities_from_registry(&registry)
}

/// Every descriptor-owned daemon system ability, independent of runtime plugin
/// installation state.
///
/// What this is NOT: the live daemon discovery surface. It deliberately builds
/// the catalogue with plugin package registration disabled so descriptor
/// generation cannot read `$HOME/.easynet/plugins` or write user-local plugin
/// descriptors by accident.
pub fn published_system_abilities() -> Vec<SystemAbilityMetadata> {
    let registry = build_system_registry();
    published_abilities_from_registry(&registry)
}

/// Published system abilities whose authority/projection class was declared as
/// `owner` in the registry.
///
/// This is the descriptor-generation path for implementation profiles.
/// Projection membership comes from `AxonAbilityCatalog::lookup_owner`, not
/// from ability name prefixes. That keeps the profile catalogue aligned with
/// the handler registration truth table and prevents broad namespaces such as
/// `device.*` from accidentally stealing abilities advertised by the
/// device-profile Agent or any hosted sub-profile Agent.
pub fn published_system_abilities_for_owner(
    owner: crate::daemon::ability::dispatch::OwnerKind,
) -> Vec<SystemAbilityMetadata> {
    let registry = build_system_registry();
    published_abilities_from_registry_for_owner(&registry, Some(&owner))
}

/// Owner declared by the deterministic system registry for one daemon-hosted
/// ability.
///
/// This is the narrow receipt/descriptor classification surface. It exposes
/// the registry's ownership truth table without letting callers depend on the
/// registry object or fall back to profile prefix matching.
pub fn system_ability_owner(
    ability_name: &str,
) -> Option<crate::daemon::ability::dispatch::OwnerKind> {
    let registry = build_system_registry();
    // SPEC §9.1.A Step 5: ownership truth comes from the control-plane
    // record, not the legacy `owner` side table (equivalence pinned by
    // `control_plane_owner_matches_legacy_lookup_for_static_ability`).
    registry.control_plane_owner(ability_name)
}

fn published_abilities_from_registry(registry: &AxonAbilityCatalog) -> Vec<SystemAbilityMetadata> {
    published_abilities_from_registry_for_owner(registry, None)
}

fn published_abilities_from_registry_for_owner(
    registry: &AxonAbilityCatalog,
    owner: Option<&crate::daemon::ability::dispatch::OwnerKind>,
) -> Vec<SystemAbilityMetadata> {
    let hint_snapshot = AbilityDiscoveryHintSnapshot::from_registry(registry);
    let catalog_snapshot = registry.ability_catalog_snapshot();
    catalog_snapshot
        .into_iter()
        .filter(|row| is_publishable_catalog_name(&row.name))
        .filter(|row| {
            owner
                // SPEC §9.1.A Step 5: owner filter reads the control-plane
                // record, not the legacy `owner` side table.
                .map(|expected| row.owner.as_ref() == Some(expected))
                .unwrap_or(true)
        })
        .map(|row| row.name)
        .filter(|name| !name.ends_with(".chat"))
        // RFC-002 §3.3 keyring abilities are owner-namespaced under
        // `device` and self-described by `keyring::abilities` — they
        // don't go through the system descriptor table. Filter them
        // for the same reason `<agent>.chat` is filtered: their
        // schema lives inside the registering module, not here.
        .filter(|name| !name.starts_with("device.keyring."))
        .map(|name| SystemAbilityMetadata {
            description: description_for_owned(&name),
            input_schema: input_schema_for(&name),
            hints: hint_snapshot.for_name(&name),
            name,
        })
        .collect()
}

/// Canonical descriptor path for a published ability.
///
/// Built-in daemon abilities resolve through the descriptor-root helper;
/// runtime plugin abilities own their descriptor TOMLs inside their package
/// directory.
pub fn descriptor_path_for(name: &str) -> String {
    crate::daemon::plugins::ability_descriptor_path(name).unwrap_or_else(|| {
        system_ability_descriptor_path(name)
            .to_string_lossy()
            .into_owned()
    })
}

/// One catalogue-local call-mode index used while rendering descriptor hints.
///
/// The previous implementation derived hints by calling
/// `registry.has_rpc/has_stream/has_bidi` for each ability. With a runtime
/// attached those helpers synchronously queried `LocalRuntime::ability_options`,
/// so a pure list operation fanned out into three async runtime reads per row.
/// This snapshot is the bounded read model: one control-plane pass, then O(1)
/// local lookups for every rendered descriptor.
#[derive(Debug, Clone)]
pub(crate) struct AbilityDiscoveryHintSnapshot {
    modes_by_ability: BTreeMap<String, BTreeSet<DescriptorCallMode>>,
}

impl AbilityDiscoveryHintSnapshot {
    pub(crate) fn from_registry(registry: &AxonAbilityCatalog) -> Self {
        Self {
            modes_by_ability: registry.call_modes_by_ability(),
        }
    }

    pub(crate) fn for_name(&self, name: &str) -> AbilityHints {
        discovery_hints_from_modes(name, self.modes_by_ability.get(name))
    }
}

#[cfg(test)]
pub(crate) fn discovery_hints_for(registry: &AxonAbilityCatalog, name: &str) -> AbilityHints {
    AbilityDiscoveryHintSnapshot::from_registry(registry).for_name(name)
}

fn discovery_hints_from_modes(
    name: &str,
    modes: Option<&BTreeSet<DescriptorCallMode>>,
) -> AbilityHints {
    if name.ends_with(".chat") {
        // Hosted chat abilities are registered on the local stream
        // surface today, but the user-facing control-plane path still
        // serves them through unary invoke + OpenAI compatibility.
        // Advertising them as `streaming_only` would regress the UI
        // into choosing InvokeStream against a daemon path that is not
        // yet wired for generic stream fallback.
        return Default::default();
    }
    if name == crate::daemon::ability::builtins::resources::media::ABILITY_CAMERA_SUBSCRIBE {
        // Camera preview has a unary compatibility path for older
        // app shells that still call Invoke for the first frame.
        // Discovery must continue to advertise the canonical stream
        // shape so correct clients choose Subscribe/InvokeStream.
        return crate::daemon::ability::descriptors::AbilityHints {
            streaming_only: true,
            ..Default::default()
        };
    }
    let has_rpc = modes.is_some_and(|modes| modes.contains(&DescriptorCallMode::Rpc));
    let has_stream = modes.is_some_and(|modes| modes.contains(&DescriptorCallMode::Stream));
    let has_bidi = modes.is_some_and(|modes| modes.contains(&DescriptorCallMode::Bidi));
    // Derive the purity hints from the ability's semantic layer — one
    // source of truth (classify_ability). Introspection/Observation are
    // pure reads (read_only + idempotent: re-issuing yields the same
    // snapshot, no side effect). Control is a pure decision (idempotent,
    // not a business read). Operational verbs change the world (neither).
    // These hints ride meta.list_abilities into the catalog, so the
    // frontend coalesces pure-read invokes from the catalog instead of
    // re-classifying ability names locally. `destructive` stays a
    // conservative false: the layer model does not assert destructiveness,
    // and the hint is advisory only (RFC §1.6).
    let (read_only, idempotent) = match classify_ability(name) {
        Some(AbilityLayer::Introspection) | Some(AbilityLayer::Observation) => (true, true),
        Some(AbilityLayer::Control) => (false, true),
        Some(AbilityLayer::Operational) | None => (false, false),
    };
    AbilityHints {
        read_only,
        idempotent,
        streaming_only: has_stream && !has_rpc && !has_bidi,
        bidi_only: has_bidi,
        ..Default::default()
    }
}

/// Human-readable description for a published system ability name.
///
/// Authoritative source for the description text. `registry::a2a_labels`
/// re-exports through this so the wire payload and the runtime
/// register call agree byte-for-byte. Falls back to a short generic
/// blurb for unknown names; the `_ if name.ends_with(".chat")` arm
/// exists because `published_ability_names()` includes per-agent chat
/// handlers when called from the daemon registry (the `published_abilities`
/// filter strips them, but other callers may not).
pub fn description_for(name: &str) -> &'static str {
    if let Some(description) = crate::daemon::plugins::description_for(name) {
        return description;
    }

    match name {
        governance_names::OBSERVE_HEALTH => ping::description(),
        governance_names::OBSERVE_NETWORK_HEALTH => network_health_ability::description(),
        device_names::SESSION_LIST => session_ability::list_description(),
        device_names::SESSION_ATTACH => session_ability::attach_description(),
        agent_names::CHAT_HISTORY_LIST => chat_history_ability::list_description(),
        agent_names::CHAT_HISTORY_GET => chat_history_ability::get_description(),
        name if name.starts_with("context.") => {
            context_ability::description_for(name).unwrap_or("Context surface ability.")
        }
        governance_names::CONSENT_SUBSCRIBE => permission_ability::subscribe_description(),
        governance_names::CONSENT_DECIDE => permission_ability::decide_description(),
        governance_names::CONSENT_LIST_PENDING => permission_ability::list_pending_description(),
        automation_names::DISCUSS_CREATE => discuss_ability::create_description(),
        automation_names::DISCUSS_POST => discuss_ability::post_description(),
        automation_names::DISCUSS_SUBSCRIBE => discuss_ability::subscribe_description(),
        automation_names::DISCUSS_LIST_TURNS => discuss_ability::list_turns_description(),
        automation_names::SCHEDULE_ADD => schedule_ability::add_description(),
        automation_names::SCHEDULE_LIST => schedule_ability::list_description(),
        automation_names::SCHEDULE_REMOVE => schedule_ability::remove_description(),
        automation_names::SCHEDULE_ENABLE => schedule_ability::enable_description(),
        automation_names::LOOP_CREATE => loop_ability::create_description(),
        automation_names::LOOP_STATUS => loop_ability::status_description(),
        automation_names::LOOP_SUBSCRIBE => loop_ability::subscribe_description(),
        automation_names::LOOP_CANCEL => loop_ability::cancel_description(),
        resource_names::SKILL_INSTALL => skill_install_ability::install_description(),
        resource_names::SKILL_REMOVE => skill_install_ability::remove_description(),
        resource_names::SKILL_UPGRADE => skill_install_ability::upgrade_description(),
        integration_names::MCP_BRIDGE_LIST_TOOLS => mcp_bridge_ability::list_tools_description(),
        integration_names::MCP_BRIDGE_CALL_TOOL => mcp_bridge_ability::call_tool_description(),
        integration_names::A2A_BRIDGE_LIST_SKILLS => a2a_bridge_ability::list_skills_description(),
        integration_names::A2A_BRIDGE_SEND_TASK => a2a_bridge_ability::send_task_description(),
        integration_names::A2A_CLIENT_SEND_TASK => a2a_client_ability::send_task_description(),
        integration_names::MCP_CLIENT_LIST => mcp_client_ability::list_description(),
        integration_names::MCP_CLIENT_CALL => mcp_client_ability::call_description(),
        agent_names::AGENT_LIST => agent_list_ability::list_agents_description(),
        plugin_lifecycle_ability::RELOAD_ABILITY => plugin_lifecycle_ability::reload_description(),
        plugin_lifecycle_ability::STATUS_ABILITY => plugin_lifecycle_ability::status_description(),
        plugin_lifecycle_ability::ACTIVATE_REALTIME_ABILITY => {
            plugin_lifecycle_ability::activate_realtime_description()
        }
        governance_names::META_DESCRIBE => meta_ability::describe_description(),
        governance_names::META_LIST_ABILITIES => meta_ability::list_abilities_description(),
        teach_ability::TEACH => teach_ability::teach_description(),
        teach_ability::ACQUIRE => teach_ability::acquire_description(),
        teach_ability::FORGET => teach_ability::forget_description(),
        automation_names::MISSION_RUN => mission_ability::run_description(),
        automation_names::MISSION_TRACK => mission_ability::track_description(),
        automation_names::MISSION_CANCEL => mission_ability::cancel_description(),
        // AXIOM §"Tier 2.5" Baseline Locomotion — filesystem half.
        device_names::FS_READ => fs_ability::description_read(),
        device_names::FS_WRITE => fs_ability::description_write(),
        device_names::FS_STAT => fs_ability::description_stat(),
        device_names::FS_LIST => fs_ability::description_list(),
        device_names::FS_EDIT => fs_edit_ability::description(),
        device_names::PROCESS_EXEC => process_exec_ability::description(),
        device_names::SHELL_RUN => shell_run_ability::description(),
        device_names::HTTP_REQUEST => http_request_ability::description(),
        governance_names::INVOCATION_HISTORY_LIST => {
            invocation_history_ability::list_history_description()
        }
        governance_names::INVOCATION_HISTORY_GET => {
            invocation_history_ability::get_history_description()
        }
        governance_names::INVOCATION_TRACE_GET => {
            invocation_history_ability::get_trace_description()
        }
        governance_names::INVOCATION_HISTORY_PATH => {
            invocation_history_ability::get_path_description()
        }
        device_names::TERMINAL_CREATE => pty_lifecycle_ability::description_create(),
        device_names::TERMINAL_LIST => pty_lifecycle_ability::description_list(),
        device_names::TERMINAL_CLOSE => pty_lifecycle_ability::description_close(),
        device_names::TERMINAL_ATTACH => pty_attach_ability::description(),
        device_names::TERMINAL_INPUT => pty_io_ability::input_description(),
        device_names::TERMINAL_READ => pty_io_ability::read_description(),
        device_names::TERMINAL_RESIZE => pty_io_ability::resize_description(),
        device_names::FS_TRANSFER => file_transfer_ability::description(),
        agent_names::AGENT_START => agent_lifecycle_ability::start_agent_description(),
        agent_names::AGENT_STOP => agent_lifecycle_ability::stop_agent_description(),
        agent_names::AGENT_REFRESH => agent_lifecycle_ability::refresh_agents_description(),
        federation_names::NODE_LIST => device_ops_ability::list_nodes_description(),
        federation_names::NODE_DESCRIBE => device_ops_ability::describe_node_description(),
        federation_names::NODE_REMOVE => device_ops_ability::remove_node_description(),
        federation_names::ABILITY_DEPLOY => device_ops_ability::deploy_ability_description(),
        federation_names::ABILITY_UNINSTALL => device_ops_ability::uninstall_ability_description(),
        automation_names::MISSION_DISCUSS_ROUND => {
            orchestration_ability::discuss_round_description()
        }
        resource_names::VOICE_CREATE_CALL => voice_call_ability::create_call_description(),
        resource_names::VOICE_SHOW_CALL => voice_call_ability::show_call_description(),
        resource_names::VOICE_JOIN_CALL => voice_call_ability::join_call_description(),
        resource_names::VOICE_LEAVE_CALL => voice_call_ability::leave_call_description(),
        resource_names::VOICE_END_CALL => voice_call_ability::end_call_description(),
        resource_names::VOICE_WATCH_CALL => voice_call_ability::watch_call_description(),
        resource_names::VOICE_REPORT_METRICS => voice_call_ability::report_metrics_description(),
        resource_names::VOICE_LIST_CALLS => voice_call_ability::list_calls_description(),
        // RFC-012 §RemoteWebSurface — browser.* family.
        device_names::BROWSER_OPEN_SESSION => browser_session_ability::open_session_description(),
        device_names::BROWSER_SEND_INPUT => browser_session_ability::send_input_description(),
        device_names::BROWSER_CAPTURE_VIEWPORT => {
            browser_session_ability::capture_viewport_description()
        }
        device_names::BROWSER_CLOSE_SESSION => browser_session_ability::close_session_description(),
        device_names::BROWSER_ATTACH_SESSION => {
            browser_session_ability::attach_session_description()
        }
        governance_names::ADMIN_STATUS => admin_status_ability::description(),
        federation_names::ABILITY_PUBLISH => ability_publish_ability::publish_description(),
        federation_names::ABILITY_UNPUBLISH => ability_publish_ability::unpublish_description(),
        resource_names::SKILL_PUBLISH => skill_publish_ability::publish_description(),
        resource_names::SKILL_UNPUBLISH => skill_publish_ability::unpublish_description(),
        resource_names::SKILL_LIST => skill_publish_ability::list_description(),
        resource_names::SKILL_TREE => skill_publish_ability::tree_description(),
        resource_names::SKILL_READ_FILE => skill_publish_ability::read_file_description(),
        resource_names::SKILL_WRITE_FILE => skill_publish_ability::write_file_description(),
        automation_names::MISSION_THINK => think_ability::description(),
        // RFC-005 v3.2 A1–A8 — media abilities. `resources::media`
        // owns the single source of truth (the `ABILITIES` table);
        // the projection here is one Option lookup, no per-name
        // arm. A 9th media ability requires touching only that
        // table; this arm picks the new name up automatically.
        n if media::description(n).is_some() => media::description(n).unwrap(),
        // RFC-005 v3.2 A9 — meta.list_resources. Lives in its own
        // module because the handler is fully real (not a stub).
        list_resources_ability::ABILITY_META_LIST_RESOURCES => {
            list_resources_ability::description()
        }
        // RFC-006-C v0.1 — device-local OpenAI protocol shim. The
        // handler runs on this host and only sees host-local
        // chat-base abilities; there is no hub round-trip in the
        // call path. Hub-side OpenAI adapters (if any realm hub
        // chooses to advertise them) live behind `hub.openai.*`,
        // queried through `federation.resolve` — the device daemon
        // never pre-registers a `hub.*` name.
        integration_names::OPENAI_CHAT_COMPLETIONS => {
            "OpenAI-compatible /v1/chat/completions served by the \
             device daemon. Requires `request.model` to be a canonical \
             agent-owned chat Ability URA, forwards the request to that \
             host-local chat-base ability (`<agent>.chat`), and then \
             projects the streaming/non-streaming reply into \
             OpenAI's response shape."
        }
        integration_names::OPENAI_LIST_MODELS => {
            "OpenAI-compatible /v1/models served by the device daemon. \
             Returns every host-local chat-base ability \
             (`<agent>.chat`) the calling identity has dispatch grants \
             on, projected as OpenAI `Model` objects whose `id` is the \
             canonical agent-owned chat Ability URA."
        }
        integration_names::OPENAI_FILES_UPLOAD => {
            "OpenAI-compatible file upload served by the device daemon. \
             Accepts Compatibility-profile file bytes, stores them in \
             the user-rooted content-addressed files surface, and \
             projects the stored blob as an OpenAI-compatible File object."
        }
        integration_names::OPENAI_FILES_RETRIEVE => {
            "OpenAI-compatible file retrieval served by the device daemon. \
             Resolves a Compatibility-profile file id through the \
             user-rooted files surface and returns file metadata plus \
             base64 content for the HTTP compatibility boundary."
        }
        integration_names::OPENAI_FILES_DELETE => {
            "OpenAI-compatible file deletion served by the device daemon. \
             Projects a deterministic logical delete acknowledgement for \
             content-addressed files whose bytes may be shared by refs."
        }
        _ if name.ends_with(".chat") => "Send a chat prompt to the locally-installed agent.",
        // `<user>.api_key.{create,list,revoke}` — user-rooted
        // credential-lifecycle abilities. `<user>` is the active
        // identity at registry-build time (uuid in prod,
        // `"test"` in fixtures); the description must match by
        // suffix rather than full name so a new user doesn't
        // silently fall through to "(system ability)".
        _ if name.ends_with(".api_key.create") => {
            "Issue a new API key for the calling user. Returns the bearer secret once; \
             the daemon stores only a hashed fingerprint."
        }
        _ if name.ends_with(".api_key.list") => {
            "List the calling user's API keys (fingerprints + metadata, no secrets)."
        }
        _ if name.ends_with(".api_key.revoke") => {
            "Revoke an API key by its fingerprint. The bearer is rejected immediately on \
             every subsequent call."
        }
        _ => "(system ability)",
    }
}

/// Owned description projection for registry publication.
///
/// Plugin packages own descriptor text that may come from TOML at runtime.
/// Builtin system abilities still use the static `description_for` table.
pub fn description_for_owned(name: &str) -> String {
    crate::daemon::plugins::builtin_description_for_owned(name)
        .unwrap_or_else(|| description_for(name).to_string())
}

/// JSON Schema for a published system ability's input. Mirrors
/// `description_for` — adding an arm here is the second half of
/// landing a new system ability so it can register against
/// axon-runtime with a real schema (not the empty-object default).
///
/// Unknown names fall back to `{"type":"object"}` — the most
/// permissive shape that still validates as a JSON Schema. A future
/// ability that lands without an arm here is callable but appears
/// as schema-less in MCP `ListTools`; a CI test pins the table
/// against the live registry to surface that drift.
pub fn input_schema_for(name: &str) -> serde_json::Value {
    if let Some(schema) = crate::daemon::plugins::builtin_input_schema_for(name) {
        return schema;
    }
    if let Some(schema) = crate::daemon::plugins::input_schema_for(name) {
        return schema;
    }

    match name {
        governance_names::OBSERVE_HEALTH => ping::input_schema(),
        governance_names::OBSERVE_NETWORK_HEALTH => network_health_ability::input_schema(),
        device_names::SESSION_LIST => session_ability::list_input_schema(),
        device_names::SESSION_ATTACH => session_ability::attach_input_schema(),
        agent_names::CHAT_HISTORY_LIST => chat_history_ability::list_input_schema(),
        agent_names::CHAT_HISTORY_GET => chat_history_ability::get_input_schema(),
        name if name.starts_with("context.") => context_ability::input_schema_for(name)
            .unwrap_or_else(|| serde_json::json!({"type": "object"})),
        governance_names::CONSENT_SUBSCRIBE => permission_ability::subscribe_input_schema(),
        governance_names::CONSENT_DECIDE => permission_ability::decide_input_schema(),
        governance_names::CONSENT_LIST_PENDING => permission_ability::list_pending_input_schema(),
        automation_names::DISCUSS_CREATE => discuss_ability::create_input_schema(),
        automation_names::DISCUSS_POST => discuss_ability::post_input_schema(),
        automation_names::DISCUSS_SUBSCRIBE => discuss_ability::subscribe_input_schema(),
        automation_names::DISCUSS_LIST_TURNS => discuss_ability::list_turns_input_schema(),
        automation_names::SCHEDULE_ADD => schedule_ability::add_input_schema(),
        automation_names::SCHEDULE_LIST => schedule_ability::list_input_schema(),
        automation_names::SCHEDULE_REMOVE => schedule_ability::remove_input_schema(),
        automation_names::SCHEDULE_ENABLE => schedule_ability::enable_input_schema(),
        automation_names::LOOP_CREATE => loop_ability::create_input_schema(),
        automation_names::LOOP_STATUS => loop_ability::status_input_schema(),
        automation_names::LOOP_SUBSCRIBE => loop_ability::subscribe_input_schema(),
        automation_names::LOOP_CANCEL => loop_ability::cancel_input_schema(),
        resource_names::SKILL_INSTALL => skill_install_ability::install_input_schema(),
        resource_names::SKILL_REMOVE => skill_install_ability::remove_input_schema(),
        resource_names::SKILL_UPGRADE => skill_install_ability::upgrade_input_schema(),
        integration_names::MCP_BRIDGE_LIST_TOOLS => mcp_bridge_ability::list_tools_input_schema(),
        integration_names::MCP_BRIDGE_CALL_TOOL => mcp_bridge_ability::call_tool_input_schema(),
        integration_names::A2A_BRIDGE_LIST_SKILLS => a2a_bridge_ability::list_skills_input_schema(),
        integration_names::A2A_BRIDGE_SEND_TASK => a2a_bridge_ability::send_task_input_schema(),
        integration_names::A2A_CLIENT_SEND_TASK => a2a_client_ability::send_task_input_schema(),
        integration_names::MCP_CLIENT_LIST => mcp_client_ability::list_input_schema(),
        integration_names::MCP_CLIENT_CALL => mcp_client_ability::call_input_schema(),
        agent_names::AGENT_LIST => agent_list_ability::list_agents_input_schema(),
        plugin_lifecycle_ability::RELOAD_ABILITY => plugin_lifecycle_ability::reload_input_schema(),
        plugin_lifecycle_ability::STATUS_ABILITY => plugin_lifecycle_ability::status_input_schema(),
        plugin_lifecycle_ability::ACTIVATE_REALTIME_ABILITY => {
            plugin_lifecycle_ability::activate_realtime_input_schema()
        }
        governance_names::META_DESCRIBE => meta_ability::describe_input_schema(),
        governance_names::META_LIST_ABILITIES => meta_ability::list_abilities_input_schema(),
        teach_ability::TEACH => teach_ability::teach_input_schema(),
        teach_ability::ACQUIRE => teach_ability::acquire_input_schema(),
        teach_ability::FORGET => teach_ability::forget_input_schema(),
        automation_names::MISSION_RUN => mission_ability::run_input_schema(),
        automation_names::MISSION_TRACK => mission_ability::track_input_schema(),
        automation_names::MISSION_CANCEL => mission_ability::cancel_input_schema(),
        // AXIOM §"Tier 2.5" Baseline Locomotion — filesystem half.
        device_names::FS_READ => fs_ability::input_schema_read(),
        device_names::FS_WRITE => fs_ability::input_schema_write(),
        device_names::FS_STAT => fs_ability::input_schema_stat(),
        device_names::FS_LIST => fs_ability::input_schema_list(),
        device_names::FS_EDIT => fs_edit_ability::input_schema(),
        device_names::PROCESS_EXEC => process_exec_ability::input_schema(),
        device_names::SHELL_RUN => shell_run_ability::input_schema(),
        device_names::HTTP_REQUEST => http_request_ability::input_schema(),
        governance_names::INVOCATION_HISTORY_LIST => {
            invocation_history_ability::list_history_input_schema()
        }
        governance_names::INVOCATION_HISTORY_GET => {
            invocation_history_ability::get_history_input_schema()
        }
        governance_names::INVOCATION_TRACE_GET => {
            invocation_history_ability::get_trace_input_schema()
        }
        governance_names::INVOCATION_HISTORY_PATH => {
            invocation_history_ability::get_path_input_schema()
        }
        device_names::TERMINAL_CREATE => pty_lifecycle_ability::input_schema_create(),
        device_names::TERMINAL_LIST => pty_lifecycle_ability::input_schema_list(),
        device_names::TERMINAL_CLOSE => pty_lifecycle_ability::input_schema_close(),
        device_names::TERMINAL_ATTACH => pty_attach_ability::input_schema(),
        device_names::TERMINAL_INPUT => pty_io_ability::input_input_schema(),
        device_names::TERMINAL_READ => pty_io_ability::read_input_schema(),
        device_names::TERMINAL_RESIZE => pty_io_ability::resize_input_schema(),
        device_names::FS_TRANSFER => file_transfer_ability::input_schema(),
        agent_names::AGENT_START => agent_lifecycle_ability::start_agent_input_schema(),
        agent_names::AGENT_STOP => agent_lifecycle_ability::stop_agent_input_schema(),
        agent_names::AGENT_REFRESH => agent_lifecycle_ability::refresh_agents_input_schema(),
        federation_names::NODE_LIST => device_ops_ability::list_nodes_input_schema(),
        federation_names::NODE_DESCRIBE => device_ops_ability::describe_node_input_schema(),
        federation_names::NODE_REMOVE => device_ops_ability::remove_node_input_schema(),
        federation_names::ABILITY_DEPLOY => device_ops_ability::deploy_ability_input_schema(),
        federation_names::ABILITY_UNINSTALL => device_ops_ability::uninstall_ability_input_schema(),
        automation_names::MISSION_DISCUSS_ROUND => {
            orchestration_ability::discuss_round_input_schema()
        }
        resource_names::VOICE_CREATE_CALL => voice_call_ability::create_call_input_schema(),
        resource_names::VOICE_SHOW_CALL => voice_call_ability::show_call_input_schema(),
        resource_names::VOICE_JOIN_CALL => voice_call_ability::join_call_input_schema(),
        resource_names::VOICE_LEAVE_CALL => voice_call_ability::leave_call_input_schema(),
        resource_names::VOICE_END_CALL => voice_call_ability::end_call_input_schema(),
        resource_names::VOICE_WATCH_CALL => voice_call_ability::watch_call_input_schema(),
        resource_names::VOICE_REPORT_METRICS => voice_call_ability::report_metrics_input_schema(),
        resource_names::VOICE_LIST_CALLS => voice_call_ability::list_calls_input_schema(),
        // RFC-012 §RemoteWebSurface — browser.* family.
        device_names::BROWSER_OPEN_SESSION => browser_session_ability::open_session_input_schema(),
        device_names::BROWSER_SEND_INPUT => browser_session_ability::send_input_input_schema(),
        device_names::BROWSER_CAPTURE_VIEWPORT => {
            browser_session_ability::capture_viewport_input_schema()
        }
        device_names::BROWSER_CLOSE_SESSION => {
            browser_session_ability::close_session_input_schema()
        }
        device_names::BROWSER_ATTACH_SESSION => {
            browser_session_ability::attach_session_input_schema()
        }
        governance_names::ADMIN_STATUS => admin_status_ability::input_schema(),
        federation_names::ABILITY_PUBLISH => ability_publish_ability::publish_input_schema(),
        federation_names::ABILITY_UNPUBLISH => ability_publish_ability::unpublish_input_schema(),
        resource_names::SKILL_PUBLISH => skill_publish_ability::publish_input_schema(),
        resource_names::SKILL_UNPUBLISH => skill_publish_ability::unpublish_input_schema(),
        resource_names::SKILL_LIST => skill_publish_ability::list_input_schema(),
        resource_names::SKILL_TREE => skill_publish_ability::tree_input_schema(),
        resource_names::SKILL_READ_FILE => skill_publish_ability::read_file_input_schema(),
        resource_names::SKILL_WRITE_FILE => skill_publish_ability::write_file_input_schema(),
        automation_names::MISSION_THINK => think_ability::input_schema(),
        // RFC-005 v3.2 A1–A8 — media abilities. Same single-source
        // -of-truth pattern as `description_for` above.
        n if media::input_schema(n).is_some() => media::input_schema(n).unwrap(),
        list_resources_ability::ABILITY_META_LIST_RESOURCES => {
            list_resources_ability::input_schema()
        }
        // RFC-006-C v0.1 — device-local OpenAI shim. Schemas mirror
        // the OpenAI request envelopes the handler accepts (chat
        // completion body, plus an `auth_token` bearer for the
        // device-local api_key store).
        integration_names::OPENAI_CHAT_COMPLETIONS => serde_json::json!({
            "type": "object",
            "required": ["request"],
            "properties": {
                "request": {
                    "type": "object",
                    "description": "OpenAI-compatible /v1/chat/completions request body. The `model` field must be a canonical agent-owned chat Ability URA.",
                    "required": ["model", "messages"],
                    "properties": {
                        "model": {
                            "type": "string",
                            "description": "Canonical agent-owned chat Ability URA, e.g. easynet:///r/easynet.run/ability/alice.codex.chat."
                        },
                        "messages": {
                            "type": "array",
                            "description": "OpenAI-compatible chat messages array."
                        },
                        "stream": {
                            "type": "boolean",
                            "description": "When true, return OpenAI-compatible streaming chunks."
                        }
                    }
                },
                "auth_token": {
                    "type": "string",
                    "description": "Bearer token bound to a `<user>.api_key` entry on this host."
                }
            }
        }),
        integration_names::OPENAI_LIST_MODELS => serde_json::json!({
            "type": "object",
            "properties": {
                "auth_token": {
                    "type": "string",
                    "description": "Bearer token bound to a `<user>.api_key` entry on this host."
                }
            }
        }),
        integration_names::OPENAI_FILES_UPLOAD => serde_json::json!({
            "type": "object",
            "required": ["purpose", "filename", "bytes_b64"],
            "properties": {
                "purpose": {
                    "type": "string",
                    "description": "OpenAI file purpose, e.g. assistants or batch."
                },
                "filename": {
                    "type": "string",
                    "description": "Client-visible file name to persist with the blob metadata."
                },
                "bytes_b64": {
                    "type": "string",
                    "description": "Standard base64-encoded file bytes."
                },
                "content_type": {
                    "type": "string",
                    "description": "Optional media type for the uploaded bytes."
                },
                "auth_token": {
                    "type": "string",
                    "description": "Bearer token bound to a `<user>.api_key` entry on this host."
                }
            }
        }),
        integration_names::OPENAI_FILES_RETRIEVE => serde_json::json!({
            "type": "object",
            "required": ["file_id"],
            "properties": {
                "file_id": {
                    "type": "string",
                    "description": "File id returned by openai.files.upload."
                },
                "filename": {
                    "type": "string",
                    "description": "Optional file name override for projected metadata."
                },
                "purpose": {
                    "type": "string",
                    "description": "Optional OpenAI file purpose for projected metadata."
                },
                "created_at": {
                    "type": "integer",
                    "description": "Optional creation timestamp to preserve in the projected file object."
                },
                "auth_token": {
                    "type": "string",
                    "description": "Bearer token bound to a `<user>.api_key` entry on this host."
                }
            }
        }),
        integration_names::OPENAI_FILES_DELETE => serde_json::json!({
            "type": "object",
            "required": ["file_id"],
            "properties": {
                "file_id": {
                    "type": "string",
                    "description": "File id returned by openai.files.upload."
                },
                "auth_token": {
                    "type": "string",
                    "description": "Bearer token bound to a `<user>.api_key` entry on this host."
                }
            }
        }),
        // `<user>.api_key.{create,list,revoke}` — see the matching
        // suffix arms in `description_for` for the rationale on
        // why these match by suffix rather than full name.
        n if n.ends_with(".api_key.create") => serde_json::json!({
            "type": "object",
            "properties": {
                "label": {
                    "type": "string",
                    "description": "Optional operator-facing label for the new key."
                }
            }
        }),
        n if n.ends_with(".api_key.list") => serde_json::json!({
            "type": "object",
            "additionalProperties": false
        }),
        n if n.ends_with(".api_key.revoke") => serde_json::json!({
            "type": "object",
            "required": ["fingerprint"],
            "properties": {
                "fingerprint": {
                    "type": "string",
                    "description": "Fingerprint of the key to revoke (from .api_key.list)."
                }
            }
        }),
        _ => serde_json::json!({ "type": "object" }),
    }
}

/// RFC-006 metadata for a published ability. Returns `None` for
/// every existing ability — they emit unchanged TOMLs and on-wire
/// descriptors. PR2 (#196) adds `Some(...)` arms for the eight
/// physical-channel abilities + meta.list_resources, declaring
/// their RFC-006 class (Stream / Query). No Transition consumer
/// exists yet; the renderer + descriptor schema support it but
/// no name returns a Transition variant in v1.
pub fn rfc006_for(name: &str) -> Option<ability_toml::Rfc006Metadata> {
    if let Some(meta) = media::rfc006(name) {
        return Some(meta);
    }
    if name == list_resources_ability::ABILITY_META_LIST_RESOURCES {
        return Some(list_resources_ability::rfc006());
    }
    None
}

/// Sync bridge so `build_registry_with_services` (sync) can call
/// `reflect_all` (async).
///
/// **Why this is allowed to self-host a runtime — unlike
/// `mcp_executor::block_on_async`.** The two bridges look symmetrical
/// but live on opposite sides of the boot/serve boundary:
///
/// * The daemon's `LocalRpcHandler` runs *inside* the gRPC server's
///   tokio runtime. The MCP executor (`mcp_executor::block_on_async`)
///   therefore MUST find an ambient runtime; the absence of one is an
///   authoring bug and we fail fast.
/// * `build_registry_with_services` runs *before* the gRPC runtime
///   is spawned — it is the daemon's synchronous bootstrap, and is
///   also called from a large body of sync unit tests
///   (`build_registry()` in `real_invoke_tests`, `publish.rs`, etc.).
///   At this call site there is no ambient runtime by design; the
///   `reflect_all` work is a one-shot `tools/list` per upstream, so
///   we mint a single-threaded runtime, drive it to completion, and
///   drop it.
///

// ── Ability semantic layer (production) ──────────────────────────
// Promoted out of the test module: the layer classification is an
// ontology property of each ability, not a test fixture. It drives
// the read_only / destructive / idempotent discovery hints that flow
// to the frontend via meta.list_abilities, so callers read purity
// from the catalog instead of re-deriving it (no parallel truth).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AbilityLayer {
    /// Pure, side-effect free, deterministic for a catalog snapshot.
    Introspection,
    /// Pure decision functions (no mutation of catalog state).
    /// `consent.decide` is the documented exception: write-only-
    /// after-decision.
    Control,
    /// Derived state only; never triggers behaviour elsewhere.
    Observation,
    /// Per-feature business verbs (chat, schedule, loop, discuss,
    /// session, skill management). Not subject to the
    /// layer-purity rules; they ARE the work.
    Operational,
}

/// Classify a published ability name by the §"three layers"
/// model. A name with no match returns `None` and the
/// completeness test below fails — forcing the author of any
/// new ability to either pick a layer or update this table.
pub(crate) fn classify_ability(name: &str) -> Option<AbilityLayer> {
    // Per-agent chat handlers are operational by definition.
    if name.ends_with(".chat") {
        return Some(AbilityLayer::Operational);
    }

    if let Some(layer) = crate::daemon::plugins::ability_layer_for(name) {
        return Some(match layer {
            crate::daemon::plugins::PluginAbilityLayer::Introspection => {
                AbilityLayer::Introspection
            }
            crate::daemon::plugins::PluginAbilityLayer::Control => AbilityLayer::Control,
            crate::daemon::plugins::PluginAbilityLayer::Observation => AbilityLayer::Observation,
            crate::daemon::plugins::PluginAbilityLayer::Operational => AbilityLayer::Operational,
        });
    }

    match name {
        // ── Introspection ───────────────────────────────────
        governance_names::META_DESCRIBE
        | governance_names::META_LIST_ABILITIES
        // `mission.track` reads the persisted run dir of a
        // prior mission.run. Pure read of derived state →
        // Introspection, same logic that puts schedule.list
        // / loop.status here.
        | automation_names::MISSION_TRACK
        | integration_names::MCP_BRIDGE_LIST_TOOLS
        // mcp.client.list — aggregate read of every configured
        // upstream MCP server's tools/list. No mutation;
        // belongs with the introspection-layer reads.
        | integration_names::MCP_CLIENT_LIST
        | integration_names::A2A_BRIDGE_LIST_SKILLS
        | agent_names::AGENT_LIST
        | governance_names::INVOCATION_HISTORY_LIST
        | governance_names::INVOCATION_HISTORY_GET
        | governance_names::INVOCATION_TRACE_GET
        | governance_names::INVOCATION_HISTORY_PATH
        | device_names::TERMINAL_LIST
        | device_names::SESSION_LIST
        | governance_names::CONSENT_LIST_PENDING
        // RFC-005 v3.2 A9 — meta.list_resources is a pure read of
        // the local resources table (same shape as
        // meta.list_abilities); Introspection by definition.
        | "meta.list_resources"
        // discuss.list_turns — RPC snapshot of a room transcript.
        // Pure read; same Introspection class as schedule.list.
        | automation_names::DISCUSS_LIST_TURNS
        | automation_names::SCHEDULE_LIST
        | automation_names::LOOP_STATUS
        // skill.list / tree / read_file — private skill package
        // inventory and source inspection. Pure reads.
        | resource_names::SKILL_LIST
        | resource_names::SKILL_TREE
        | resource_names::SKILL_READ_FILE
        // chat.history.* — pure reads of persisted chat
        // transcripts (JSONL under the agent workspace). Same
        // Introspection class as invocation.history.*.
        | agent_names::CHAT_HISTORY_LIST
        | agent_names::CHAT_HISTORY_GET
        // context.* reads — clipboard history, mapped-folder
        // browse, favorites, and persisted media captures are
        // all pure reads of device-local context state.
        | "context.clipboard.list"
        | "context.clipboard.get"
        | "context.folders.list"
        | "context.fs.list"
        | "context.favorites.list"
        | "context.captures.list"
        | "context.captures.get" => Some(AbilityLayer::Introspection),
        // ── Control / decision ──────────────────────────────
        governance_names::CONSENT_DECIDE
        // context mutations — flip clipboard tracking, delete a
        // clip, add / remove favorites: device-context
        // configuration writes, same decision class as
        // consent.decide.
        | "context.clipboard.track"
        | "context.clipboard.remove"
        | "context.favorites.add"
        | "context.favorites.remove"
        | governance_names::CONSENT_SUBSCRIBE => Some(AbilityLayer::Control),
        // ── Observation ─────────────────────────────────────
        governance_names::OBSERVE_HEALTH
        | governance_names::OBSERVE_NETWORK_HEALTH
        | governance_names::ADMIN_STATUS
        | "plugin.status" => Some(AbilityLayer::Observation),
        // ── Operational (per-feature business verbs) ────────
        device_names::SESSION_ATTACH
        | agent_names::AGENT_START
        | agent_names::AGENT_STOP
        | agent_names::AGENT_REFRESH
        | resource_names::SKILL_INSTALL
        | resource_names::SKILL_REMOVE
        | resource_names::SKILL_UPGRADE
        // device-hosted node/ability/remote operations. list_nodes /
        // describe_node read state but conceptually they sit
        // with the federation-tier *operations* (peer
        // enumeration, network health) — Operational by
        // intent, mirroring how schedule.list / loop.status
        // got bumped into the introspection layer because they
        // describe daemon-managed state. The remaining
        // verbs (remove_node, deploy_ability, uninstall_ability)
        // mutate state — Operational unambiguous.
        | federation_names::NODE_LIST
        | federation_names::NODE_DESCRIBE
        | federation_names::NODE_REMOVE
        | federation_names::ABILITY_DEPLOY
        | federation_names::ABILITY_UNINSTALL
        // terminal.* shell-session lifecycle abilities.
        // create / close mutate session state; input / read /
        // resize push or pull data over an established session;
        // attach binds the bidi data plane. All operational
        // because each call IS the work for that session step.
        | device_names::TERMINAL_ATTACH
        | device_names::TERMINAL_CREATE
        | device_names::TERMINAL_CLOSE
        | device_names::TERMINAL_INPUT
        | device_names::TERMINAL_READ
        | device_names::TERMINAL_RESIZE
        // mission.discuss_round — sub-turn orchestration
        // ability. Same Operational class as easynet.run /
        // mission.run because the ability IS the work
        // (running one human-bracketed sub-turn of a
        // multi-agent discussion).
        | automation_names::MISSION_DISCUSS_ROUND
        // mission.think — long-running worker+judge loop. Same
        // Operational rationale: the ability IS the work
        // (running an N-cycle reflective loop with two
        // independent chat sessions).
        | automation_names::MISSION_THINK
        // voice.* call signaling abilities. State-mutating
        // (create / join / leave / end / report_metrics) and
        // state-reading (show / watch) — Operational by intent
        // because the call IS the work. Same shape as
        // discuss.subscribe / loop.subscribe sit here.
        | resource_names::VOICE_CREATE_CALL
        | resource_names::VOICE_SHOW_CALL
        | resource_names::VOICE_JOIN_CALL
        | resource_names::VOICE_LEAVE_CALL
        | resource_names::VOICE_END_CALL
        | resource_names::VOICE_WATCH_CALL
        | resource_names::VOICE_REPORT_METRICS
        | resource_names::VOICE_LIST_CALLS
        // mcp.bridge.call_tool / a2a.bridge.send_task — both
        // dispatch into another local ability; the side effects
        // come from that dispatch, not the bridge itself. Sit
        // with the operational verbs because the call surface
        // IS the work.
        | integration_names::MCP_BRIDGE_CALL_TOOL
        // mcp.client.call — outbound mirror of bridge.call_tool.
        // Same operational classification: dispatching
        // delegates side effects to the upstream tool.
        | integration_names::MCP_CLIENT_CALL
        | integration_names::A2A_BRIDGE_SEND_TASK
        // a2a.client.send_task — outbound mirror of bridge.send_task.
        // Same operational classification: dispatching crosses
        // a wire and mutates the remote node's state.
        | integration_names::A2A_CLIENT_SEND_TASK
        | automation_names::DISCUSS_CREATE
        | automation_names::DISCUSS_POST
        | automation_names::DISCUSS_SUBSCRIBE
        | automation_names::SCHEDULE_ADD
        | automation_names::SCHEDULE_REMOVE
        | automation_names::SCHEDULE_ENABLE
        | automation_names::LOOP_CREATE
        | automation_names::LOOP_SUBSCRIBE
        | automation_names::LOOP_CANCEL
        // EAL orchestration. easynet.run / mission.run compile
        // and execute a program (potentially multi-step,
        // potentially cross-agent); easynet.cancel mutates the
        // run state of an in-flight mission. Same Operational
        // class as loop.{create,cancel} for the same reason —
        // the ability IS the work.
        | automation_names::MISSION_RUN
        | automation_names::MISSION_CANCEL
        // ability.publish / ability.unpublish / skill.publish /
        // skill.unpublish — curator-driven sinks for judge-validated
        // experience. State-mutating (writes/removes manifests under
        // an agent's workspace). Operational because the ability IS
        // the work, in the same class as ability.deploy /
        // skill.install.
        | federation_names::ABILITY_PUBLISH
        | federation_names::ABILITY_UNPUBLISH
        | "meta.teach"
        | "meta.acquire"
        | "meta.forget"
        | resource_names::SKILL_PUBLISH
        | resource_names::SKILL_UNPUBLISH
        | resource_names::SKILL_WRITE_FILE
        // AXIOM §"Tier 2.5" Baseline Locomotion Profile,
        // filesystem half. fs.read is technically read-only
        // but it returns business content, not just metadata
        // — Operational rather than Observation. fs.write
        // mutates state. fs.list returns directory metadata
        // but its purpose is to enable subsequent fs.read /
        // fs.write — Operational by intent.
        | device_names::FS_READ
        | device_names::FS_WRITE
        | device_names::FS_STAT
        | device_names::FS_LIST
        | device_names::FS_EDIT
        // AXIOM Tier 2.5 execution members. process.exec
        // and shell.run are unconditionally Operational —
        // they spawn processes that may do anything; even
        // with the 8-stage shellguard pipeline gating
        // shell.run dispatch, the layer classification
        // tracks privilege not invocation safety.
        | device_names::PROCESS_EXEC
        | device_names::SHELL_RUN
        | device_names::HTTP_REQUEST
        | device_names::FS_TRANSFER
        // RFC-005 v3.2 A1–A8 — physical-channel media verbs.
        // Operational by intent: each one drives an external
        // device (mic / camera / speaker / screen) or remote
        // model (voice / asr). Subject = resource_ura.
        | "mic.subscribe"
        | "camera.subscribe"
        | "camera.snapshot"
        | "camera.record_start"
        | "camera.record_stop"
        | "screen.subscribe"
        | "screen.snapshot"
        | "speaker.publish"
        | "voice.subscribe"
        | "voice.transcribe"
        // RFC-006-C v0.1 — device-local OpenAI protocol shim.
        // chat_completions IS the work (forwards a generation
        // request to a host-local chat-base ability);
        // list_models reads the caller's dispatch-grant set,
        // but its operational role is "answer /v1/models for
        // the OpenAI surface" — both are Operational rather
        // than Introspection.
        | integration_names::OPENAI_CHAT_COMPLETIONS
        | integration_names::OPENAI_LIST_MODELS
        | integration_names::OPENAI_FILES_UPLOAD
        | integration_names::OPENAI_FILES_RETRIEVE
        | integration_names::OPENAI_FILES_DELETE
        // RFC-012 §RemoteWebSurface — browser.* family.
        // Operational by intent: opening a WebView session,
        // streaming frames, injecting input, closing the
        // session all drive an external surface (the user's
        // system WebView) under the caller's identity. Same
        // class as media/* verbs.
        | device_names::BROWSER_OPEN_SESSION
        | device_names::BROWSER_ATTACH_SESSION
        | device_names::BROWSER_SEND_INPUT
        | device_names::BROWSER_CAPTURE_VIEWPORT
        | device_names::BROWSER_CLOSE_SESSION
        // Plugin lifecycle reload mutates the daemon's dynamic
        // ability registration table after an install/update/remove
        // transaction has already committed on disk.
        | "plugin.reload"
        | "plugin.activate_realtime"
        => Some(AbilityLayer::Operational),
        // `<user>.api_key.{create,list,revoke}` — user-rooted
        // credential-lifecycle verbs. `<user>` is the active
        // identity (uuid in prod, `"test"` in fixtures), so we
        // match by suffix rather than enumerating one identity.
        // All three are Operational because the ability IS the
        // work (issuing / listing / revoking a credential), in
        // the same class as ability.publish / skill.publish.
        n if n.ends_with(".api_key.create")
            || n.ends_with(".api_key.list")
            || n.ends_with(".api_key.revoke") =>
        {
            Some(AbilityLayer::Operational)
        }
        _ => None,
    }
}
