// EasyNet CLI — published-ability catalogue metadata
// ==================================================
//
// The read-only descriptor surface: published names/metadata,
// descriptor paths, descriptions, input schemas, RFC-006 rows.
// Split from agents/mod.rs (F-027 / T4.5); bodies are move-only.

use super::{
    a2a_bridge_ability, a2a_client_ability, ability_publish_ability, ability_toml,
    admin_status_ability, agent_lifecycle_ability, agent_list_ability, browser_session_ability,
    build_registry, chat_history_ability, context_ability, device_ops_ability, discover_ability,
    discuss_ability, file_transfer_ability, fs_ability, fs_edit_ability, http_request_ability,
    invocation_history_ability, list_resources_ability, loop_ability, mcp_bridge_ability,
    mcp_client_ability, media_abilities, meta_ability, mission_ability, network_health_ability,
    orchestration_ability, permission_ability, ping, plugin_lifecycle_ability,
    process_exec_ability, pty_attach_ability, pty_io_ability, pty_lifecycle_ability,
    registry_builder::build_system_registry, schedule_ability, session_ability, shell_run_ability,
    skill_install_ability, skill_publish_ability, teach_ability, think_ability, voice_call_ability,
};
use crate::runtime::ability_dispatch::AxonAbilityCatalog;

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
/// runtime-local register publisher (`runtime::publish`), and any
/// future `easynet ability list --system` surface — pulls from one
/// table. Adding a new system ability now requires updating exactly
/// one match arm in `metadata_for`; previously the same name lived
/// in three places that could (and did) drift.
#[derive(Debug, Clone)]
pub struct SystemAbilityMetadata {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub hints: crate::runtime::ability_descriptor::AbilityHints,
}

/// Every published system ability's metadata, in the deterministic
/// order `published_ability_names()` returns.
///
/// `<agent>.chat` is **excluded** even when the live registry would
/// include it: those entries are already published to the
/// axon-runtime via `runtime::publish::republish_abilities_via_advertise`
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
    owner: crate::runtime::ability_dispatch::OwnerKind,
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
) -> Option<crate::runtime::ability_dispatch::OwnerKind> {
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
    owner: Option<&crate::runtime::ability_dispatch::OwnerKind>,
) -> Vec<SystemAbilityMetadata> {
    registry
        .list_abilities()
        .into_iter()
        .filter(|name| is_publishable_catalog_name(name))
        .filter(|name| {
            owner
                // SPEC §9.1.A Step 5: owner filter reads the control-plane
                // record, not the legacy `owner` side table.
                .map(|expected| registry.control_plane_owner(name).as_ref() == Some(expected))
                .unwrap_or(true)
        })
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
            hints: discovery_hints_for(registry, &name),
            name,
        })
        .collect()
}

/// Canonical descriptor path for a published ability.
///
/// Built-in daemon abilities remain under `abilities/system`; runtime plugin
/// abilities own their descriptor TOMLs inside their package directory.
pub fn descriptor_path_for(name: &str) -> String {
    crate::runtime::plugin_host::ability_descriptor_path(name)
        .unwrap_or_else(|| format!("abilities/system/{name}.ability.toml"))
}

pub(crate) fn discovery_hints_for(
    registry: &crate::runtime::ability_dispatch::AxonAbilityCatalog,
    name: &str,
) -> crate::runtime::ability_descriptor::AbilityHints {
    if name.ends_with(".chat") {
        // Hosted chat abilities are registered on the local stream
        // surface today, but the user-facing control-plane path still
        // serves them through unary invoke + OpenAI compatibility.
        // Advertising them as `streaming_only` would regress the UI
        // into choosing InvokeStream against a daemon path that is not
        // yet wired for generic stream fallback.
        return Default::default();
    }
    if name == crate::runtime::agents::media_abilities::ABILITY_CAMERA_SUBSCRIBE {
        // Camera preview has a unary compatibility path for older
        // app shells that still call Invoke for the first frame.
        // Discovery must continue to advertise the canonical stream
        // shape so correct clients choose Subscribe/InvokeStream.
        return crate::runtime::ability_descriptor::AbilityHints {
            streaming_only: true,
            ..Default::default()
        };
    }
    let has_rpc = registry.has_rpc(name);
    let has_stream = registry.has_stream(name);
    let has_bidi = registry.has_bidi(name);
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
    crate::runtime::ability_descriptor::AbilityHints {
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
    if let Some(description) = crate::runtime::plugin_host::description_for(name) {
        return description;
    }

    match name {
        "observe.health" => ping::description(),
        "observe.network_health" => network_health_ability::description(),
        "session.list" => session_ability::list_description(),
        "session.attach" => session_ability::attach_description(),
        "chat.history.list" => chat_history_ability::list_description(),
        "chat.history.get" => chat_history_ability::get_description(),
        name if name.starts_with("context.") => {
            context_ability::description_for(name).unwrap_or("Context surface ability.")
        }
        "consent.subscribe" => permission_ability::subscribe_description(),
        "consent.decide" => permission_ability::decide_description(),
        "consent.list_pending" => permission_ability::list_pending_description(),
        "discuss.create" => discuss_ability::create_description(),
        "discuss.post" => discuss_ability::post_description(),
        "discuss.subscribe" => discuss_ability::subscribe_description(),
        "discuss.list_turns" => discuss_ability::list_turns_description(),
        "schedule.add" => schedule_ability::add_description(),
        "schedule.list" => schedule_ability::list_description(),
        "schedule.remove" => schedule_ability::remove_description(),
        "schedule.enable" => schedule_ability::enable_description(),
        "loop.create" => loop_ability::create_description(),
        "loop.status" => loop_ability::status_description(),
        "loop.subscribe" => loop_ability::subscribe_description(),
        "loop.cancel" => loop_ability::cancel_description(),
        "skill.install" => skill_install_ability::install_description(),
        "skill.remove" => skill_install_ability::remove_description(),
        "skill.upgrade" => skill_install_ability::upgrade_description(),
        "mcp.bridge.list_tools" => mcp_bridge_ability::list_tools_description(),
        "mcp.bridge.call_tool" => mcp_bridge_ability::call_tool_description(),
        "a2a.bridge.list_skills" => a2a_bridge_ability::list_skills_description(),
        "a2a.bridge.send_task" => a2a_bridge_ability::send_task_description(),
        "a2a.client.send_task" => a2a_client_ability::send_task_description(),
        "mcp.client.list" => mcp_client_ability::list_description(),
        "mcp.client.call" => mcp_client_ability::call_description(),
        "agent.list" => agent_list_ability::list_agents_description(),
        plugin_lifecycle_ability::RELOAD_ABILITY => plugin_lifecycle_ability::reload_description(),
        plugin_lifecycle_ability::STATUS_ABILITY => plugin_lifecycle_ability::status_description(),
        "meta.describe" => meta_ability::describe_description(),
        "meta.list_abilities" => meta_ability::list_abilities_description(),
        teach_ability::TEACH => teach_ability::teach_description(),
        teach_ability::ACQUIRE => teach_ability::acquire_description(),
        teach_ability::FORGET => teach_ability::forget_description(),
        "mission.run" => mission_ability::run_description(),
        "mission.track" => mission_ability::track_description(),
        "mission.cancel" => mission_ability::cancel_description(),
        // AXIOM §"Tier 2.5" Baseline Locomotion — filesystem half.
        "fs.read" => fs_ability::description_read(),
        "fs.write" => fs_ability::description_write(),
        "fs.stat" => fs_ability::description_stat(),
        "fs.list" => fs_ability::description_list(),
        "fs.edit" => fs_edit_ability::description(),
        "process.exec" => process_exec_ability::description(),
        "shell.run" => shell_run_ability::description(),
        "http.request" => http_request_ability::description(),
        "invocation.history.list" => invocation_history_ability::list_history_description(),
        "invocation.history.get" => invocation_history_ability::get_history_description(),
        "invocation.trace.get" => invocation_history_ability::get_trace_description(),
        "invocation.history.path" => invocation_history_ability::get_path_description(),
        "terminal.create" => pty_lifecycle_ability::description_create(),
        "terminal.list" => pty_lifecycle_ability::description_list(),
        "terminal.close" => pty_lifecycle_ability::description_close(),
        "terminal.attach" => pty_attach_ability::description(),
        "terminal.input" => pty_io_ability::input_description(),
        "terminal.read" => pty_io_ability::read_description(),
        "terminal.resize" => pty_io_ability::resize_description(),
        "fs.transfer" => file_transfer_ability::description(),
        "agent.start" => agent_lifecycle_ability::start_agent_description(),
        "agent.stop" => agent_lifecycle_ability::stop_agent_description(),
        "agent.refresh" => agent_lifecycle_ability::refresh_agents_description(),
        "node.list" => device_ops_ability::list_nodes_description(),
        "node.describe" => device_ops_ability::describe_node_description(),
        "node.remove" => device_ops_ability::remove_node_description(),
        "ability.deploy" => device_ops_ability::deploy_ability_description(),
        "ability.uninstall" => device_ops_ability::uninstall_ability_description(),
        "mission.discuss_round" => orchestration_ability::discuss_round_description(),
        "voice.create_call" => voice_call_ability::create_call_description(),
        "voice.show_call" => voice_call_ability::show_call_description(),
        "voice.join_call" => voice_call_ability::join_call_description(),
        "voice.leave_call" => voice_call_ability::leave_call_description(),
        "voice.end_call" => voice_call_ability::end_call_description(),
        "voice.watch_call" => voice_call_ability::watch_call_description(),
        "voice.report_metrics" => voice_call_ability::report_metrics_description(),
        "voice.list_calls" => voice_call_ability::list_calls_description(),
        // RFC-012 §RemoteWebSurface — browser.* family.
        "browser.open_session" => browser_session_ability::open_session_description(),
        "browser.send_input" => browser_session_ability::send_input_description(),
        "browser.capture_viewport" => browser_session_ability::capture_viewport_description(),
        "browser.close_session" => browser_session_ability::close_session_description(),
        "browser.attach_session" => browser_session_ability::attach_session_description(),
        "admin.status" => admin_status_ability::description(),
        "ability.publish" => ability_publish_ability::publish_description(),
        "ability.unpublish" => ability_publish_ability::unpublish_description(),
        "skill.publish" => skill_publish_ability::publish_description(),
        "skill.unpublish" => skill_publish_ability::unpublish_description(),
        "skill.list" => skill_publish_ability::list_description(),
        "skill.tree" => skill_publish_ability::tree_description(),
        "skill.read_file" => skill_publish_ability::read_file_description(),
        "skill.write_file" => skill_publish_ability::write_file_description(),
        "mission.think" => think_ability::description(),
        // RFC-005 v3.2 A1–A8 — media abilities. `media_abilities`
        // owns the single source of truth (the `ABILITIES` table);
        // the projection here is one Option lookup, no per-name
        // arm. A 9th media ability requires touching only that
        // table; this arm picks the new name up automatically.
        n if media_abilities::description(n).is_some() => media_abilities::description(n).unwrap(),
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
        "openai.chat_completions" => {
            "OpenAI-compatible /v1/chat/completions served by the \
             device daemon. Requires `request.model` to be a canonical \
             agent-owned chat Ability URA, forwards the request to that \
             host-local chat-base ability (`<agent>.chat`), and then \
             projects the streaming/non-streaming reply into \
             OpenAI's response shape."
        }
        "openai.list_models" => {
            "OpenAI-compatible /v1/models served by the device daemon. \
             Returns every host-local chat-base ability \
             (`<agent>.chat`) the calling identity has dispatch grants \
             on, projected as OpenAI `Model` objects whose `id` is the \
             canonical agent-owned chat Ability URA."
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
    crate::runtime::plugin_host::builtin_description_for_owned(name)
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
    if let Some(schema) = crate::runtime::plugin_host::builtin_input_schema_for(name) {
        return schema;
    }
    if let Some(schema) = crate::runtime::plugin_host::input_schema_for(name) {
        return schema;
    }

    match name {
        "observe.health" => ping::input_schema(),
        "observe.network_health" => network_health_ability::input_schema(),
        "session.list" => session_ability::list_input_schema(),
        "session.attach" => session_ability::attach_input_schema(),
        "chat.history.list" => chat_history_ability::list_input_schema(),
        "chat.history.get" => chat_history_ability::get_input_schema(),
        name if name.starts_with("context.") => context_ability::input_schema_for(name)
            .unwrap_or_else(|| serde_json::json!({"type": "object"})),
        "consent.subscribe" => permission_ability::subscribe_input_schema(),
        "consent.decide" => permission_ability::decide_input_schema(),
        "consent.list_pending" => permission_ability::list_pending_input_schema(),
        "discuss.create" => discuss_ability::create_input_schema(),
        "discuss.post" => discuss_ability::post_input_schema(),
        "discuss.subscribe" => discuss_ability::subscribe_input_schema(),
        "discuss.list_turns" => discuss_ability::list_turns_input_schema(),
        "schedule.add" => schedule_ability::add_input_schema(),
        "schedule.list" => schedule_ability::list_input_schema(),
        "schedule.remove" => schedule_ability::remove_input_schema(),
        "schedule.enable" => schedule_ability::enable_input_schema(),
        "loop.create" => loop_ability::create_input_schema(),
        "loop.status" => loop_ability::status_input_schema(),
        "loop.subscribe" => loop_ability::subscribe_input_schema(),
        "loop.cancel" => loop_ability::cancel_input_schema(),
        "skill.install" => skill_install_ability::install_input_schema(),
        "skill.remove" => skill_install_ability::remove_input_schema(),
        "skill.upgrade" => skill_install_ability::upgrade_input_schema(),
        "mcp.bridge.list_tools" => mcp_bridge_ability::list_tools_input_schema(),
        "mcp.bridge.call_tool" => mcp_bridge_ability::call_tool_input_schema(),
        "a2a.bridge.list_skills" => a2a_bridge_ability::list_skills_input_schema(),
        "a2a.bridge.send_task" => a2a_bridge_ability::send_task_input_schema(),
        "a2a.client.send_task" => a2a_client_ability::send_task_input_schema(),
        "mcp.client.list" => mcp_client_ability::list_input_schema(),
        "mcp.client.call" => mcp_client_ability::call_input_schema(),
        "agent.list" => agent_list_ability::list_agents_input_schema(),
        plugin_lifecycle_ability::RELOAD_ABILITY => plugin_lifecycle_ability::reload_input_schema(),
        plugin_lifecycle_ability::STATUS_ABILITY => plugin_lifecycle_ability::status_input_schema(),
        "meta.describe" => meta_ability::describe_input_schema(),
        "meta.list_abilities" => meta_ability::list_abilities_input_schema(),
        teach_ability::TEACH => teach_ability::teach_input_schema(),
        teach_ability::ACQUIRE => teach_ability::acquire_input_schema(),
        teach_ability::FORGET => teach_ability::forget_input_schema(),
        "mission.run" => mission_ability::run_input_schema(),
        "mission.track" => mission_ability::track_input_schema(),
        "mission.cancel" => mission_ability::cancel_input_schema(),
        // AXIOM §"Tier 2.5" Baseline Locomotion — filesystem half.
        "fs.read" => fs_ability::input_schema_read(),
        "fs.write" => fs_ability::input_schema_write(),
        "fs.stat" => fs_ability::input_schema_stat(),
        "fs.list" => fs_ability::input_schema_list(),
        "fs.edit" => fs_edit_ability::input_schema(),
        "process.exec" => process_exec_ability::input_schema(),
        "shell.run" => shell_run_ability::input_schema(),
        "http.request" => http_request_ability::input_schema(),
        "invocation.history.list" => invocation_history_ability::list_history_input_schema(),
        "invocation.history.get" => invocation_history_ability::get_history_input_schema(),
        "invocation.trace.get" => invocation_history_ability::get_trace_input_schema(),
        "invocation.history.path" => invocation_history_ability::get_path_input_schema(),
        "terminal.create" => pty_lifecycle_ability::input_schema_create(),
        "terminal.list" => pty_lifecycle_ability::input_schema_list(),
        "terminal.close" => pty_lifecycle_ability::input_schema_close(),
        "terminal.attach" => pty_attach_ability::input_schema(),
        "terminal.input" => pty_io_ability::input_input_schema(),
        "terminal.read" => pty_io_ability::read_input_schema(),
        "terminal.resize" => pty_io_ability::resize_input_schema(),
        "fs.transfer" => file_transfer_ability::input_schema(),
        "agent.start" => agent_lifecycle_ability::start_agent_input_schema(),
        "agent.stop" => agent_lifecycle_ability::stop_agent_input_schema(),
        "agent.refresh" => agent_lifecycle_ability::refresh_agents_input_schema(),
        "node.list" => device_ops_ability::list_nodes_input_schema(),
        "node.describe" => device_ops_ability::describe_node_input_schema(),
        "node.remove" => device_ops_ability::remove_node_input_schema(),
        "ability.deploy" => device_ops_ability::deploy_ability_input_schema(),
        "ability.uninstall" => device_ops_ability::uninstall_ability_input_schema(),
        "mission.discuss_round" => orchestration_ability::discuss_round_input_schema(),
        "voice.create_call" => voice_call_ability::create_call_input_schema(),
        "voice.show_call" => voice_call_ability::show_call_input_schema(),
        "voice.join_call" => voice_call_ability::join_call_input_schema(),
        "voice.leave_call" => voice_call_ability::leave_call_input_schema(),
        "voice.end_call" => voice_call_ability::end_call_input_schema(),
        "voice.watch_call" => voice_call_ability::watch_call_input_schema(),
        "voice.report_metrics" => voice_call_ability::report_metrics_input_schema(),
        "voice.list_calls" => voice_call_ability::list_calls_input_schema(),
        // RFC-012 §RemoteWebSurface — browser.* family.
        "browser.open_session" => browser_session_ability::open_session_input_schema(),
        "browser.send_input" => browser_session_ability::send_input_input_schema(),
        "browser.capture_viewport" => browser_session_ability::capture_viewport_input_schema(),
        "browser.close_session" => browser_session_ability::close_session_input_schema(),
        "browser.attach_session" => browser_session_ability::attach_session_input_schema(),
        "admin.status" => admin_status_ability::input_schema(),
        "ability.publish" => ability_publish_ability::publish_input_schema(),
        "ability.unpublish" => ability_publish_ability::unpublish_input_schema(),
        "skill.publish" => skill_publish_ability::publish_input_schema(),
        "skill.unpublish" => skill_publish_ability::unpublish_input_schema(),
        "skill.list" => skill_publish_ability::list_input_schema(),
        "skill.tree" => skill_publish_ability::tree_input_schema(),
        "skill.read_file" => skill_publish_ability::read_file_input_schema(),
        "skill.write_file" => skill_publish_ability::write_file_input_schema(),
        "mission.think" => think_ability::input_schema(),
        // RFC-005 v3.2 A1–A8 — media abilities. Same single-source
        // -of-truth pattern as `description_for` above.
        n if media_abilities::input_schema(n).is_some() => {
            media_abilities::input_schema(n).unwrap()
        }
        list_resources_ability::ABILITY_META_LIST_RESOURCES => {
            list_resources_ability::input_schema()
        }
        // RFC-006-C v0.1 — device-local OpenAI shim. Schemas mirror
        // the OpenAI request envelopes the handler accepts (chat
        // completion body, plus an `auth_token` bearer for the
        // device-local api_key store).
        "openai.chat_completions" => serde_json::json!({
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
        "openai.list_models" => serde_json::json!({
            "type": "object",
            "properties": {
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
    if let Some(meta) = media_abilities::rfc006(name) {
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

    if let Some(layer) = crate::runtime::plugin_host::ability_layer_for(name) {
        return Some(match layer {
            crate::runtime::plugin_host::PluginAbilityLayer::Introspection => {
                AbilityLayer::Introspection
            }
            crate::runtime::plugin_host::PluginAbilityLayer::Control => AbilityLayer::Control,
            crate::runtime::plugin_host::PluginAbilityLayer::Observation => {
                AbilityLayer::Observation
            }
            crate::runtime::plugin_host::PluginAbilityLayer::Operational => {
                AbilityLayer::Operational
            }
        });
    }

    match name {
        // ── Introspection ───────────────────────────────────
        "meta.describe"
        | "meta.list_abilities"
        // `mission.track` reads the persisted run dir of a
        // prior mission.run. Pure read of derived state →
        // Introspection, same logic that puts schedule.list
        // / loop.status here.
        | "mission.track"
        | "mcp.bridge.list_tools"
        // mcp.client.list — aggregate read of every configured
        // upstream MCP server's tools/list. No mutation;
        // belongs with the introspection-layer reads.
        | "mcp.client.list"
        | "a2a.bridge.list_skills"
        | "agent.list"
        | "invocation.history.list"
        | "invocation.history.get"
        | "invocation.trace.get"
        | "invocation.history.path"
        | "terminal.list"
        | "session.list"
        | "consent.list_pending"
        // RFC-005 v3.2 A9 — meta.list_resources is a pure read of
        // the local resources table (same shape as
        // meta.list_abilities); Introspection by definition.
        | "meta.list_resources"
        // discuss.list_turns — RPC snapshot of a room transcript.
        // Pure read; same Introspection class as schedule.list.
        | "discuss.list_turns"
        | "schedule.list"
        | "loop.status"
        // skill.list / tree / read_file — private skill package
        // inventory and source inspection. Pure reads.
        | "skill.list"
        | "skill.tree"
        | "skill.read_file"
        // chat.history.* — pure reads of persisted chat
        // transcripts (JSONL under the agent workspace). Same
        // Introspection class as invocation.history.*.
        | "chat.history.list"
        | "chat.history.get"
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
        "consent.decide"
        // context mutations — flip clipboard tracking, delete a
        // clip, add / remove favorites: device-context
        // configuration writes, same decision class as
        // consent.decide.
        | "context.clipboard.track"
        | "context.clipboard.remove"
        | "context.favorites.add"
        | "context.favorites.remove"
        | "consent.subscribe" => Some(AbilityLayer::Control),
        // ── Observation ─────────────────────────────────────
        "observe.health"
        | "observe.network_health"
        | "admin.status"
        | "plugin.status" => Some(AbilityLayer::Observation),
        // ── Operational (per-feature business verbs) ────────
        "session.attach"
        | "agent.start"
        | "agent.stop"
        | "agent.refresh"
        | "skill.install"
        | "skill.remove"
        | "skill.upgrade"
        // device-hosted node/ability/remote operations. list_nodes /
        // describe_node read state but conceptually they sit
        // with the federation-tier *operations* (peer
        // enumeration, network health) — Operational by
        // intent, mirroring how schedule.list / loop.status
        // got bumped into the introspection layer because they
        // describe daemon-managed state. The remaining
        // verbs (remove_node, deploy_ability, uninstall_ability)
        // mutate state — Operational unambiguous.
        | "node.list"
        | "node.describe"
        | "node.remove"
        | "ability.deploy"
        | "ability.uninstall"
        // terminal.* shell-session lifecycle abilities.
        // create / close mutate session state; input / read /
        // resize push or pull data over an established session;
        // attach binds the bidi data plane. All operational
        // because each call IS the work for that session step.
        | "terminal.attach"
        | "terminal.create"
        | "terminal.close"
        | "terminal.input"
        | "terminal.read"
        | "terminal.resize"
        // mission.discuss_round — sub-turn orchestration
        // ability. Same Operational class as easynet.run /
        // mission.run because the ability IS the work
        // (running one human-bracketed sub-turn of a
        // multi-agent discussion).
        | "mission.discuss_round"
        // mission.think — long-running worker+judge loop. Same
        // Operational rationale: the ability IS the work
        // (running an N-cycle reflective loop with two
        // independent chat sessions).
        | "mission.think"
        // voice.* call signaling abilities. State-mutating
        // (create / join / leave / end / report_metrics) and
        // state-reading (show / watch) — Operational by intent
        // because the call IS the work. Same shape as
        // discuss.subscribe / loop.subscribe sit here.
        | "voice.create_call"
        | "voice.show_call"
        | "voice.join_call"
        | "voice.leave_call"
        | "voice.end_call"
        | "voice.watch_call"
        | "voice.report_metrics"
        | "voice.list_calls"
        // mcp.bridge.call_tool / a2a.bridge.send_task — both
        // dispatch into another local ability; the side effects
        // come from that dispatch, not the bridge itself. Sit
        // with the operational verbs because the call surface
        // IS the work.
        | "mcp.bridge.call_tool"
        // mcp.client.call — outbound mirror of bridge.call_tool.
        // Same operational classification: dispatching
        // delegates side effects to the upstream tool.
        | "mcp.client.call"
        | "a2a.bridge.send_task"
        // a2a.client.send_task — outbound mirror of bridge.send_task.
        // Same operational classification: dispatching crosses
        // a wire and mutates the remote node's state.
        | "a2a.client.send_task"
        | "discuss.create"
        | "discuss.post"
        | "discuss.subscribe"
        | "schedule.add"
        | "schedule.remove"
        | "schedule.enable"
        | "loop.create"
        | "loop.subscribe"
        | "loop.cancel"
        // EAL orchestration. easynet.run / mission.run compile
        // and execute a program (potentially multi-step,
        // potentially cross-agent); easynet.cancel mutates the
        // run state of an in-flight mission. Same Operational
        // class as loop.{create,cancel} for the same reason —
        // the ability IS the work.
        | "mission.run"
        | "mission.cancel"
        // ability.publish / ability.unpublish / skill.publish /
        // skill.unpublish — curator-driven sinks for judge-validated
        // experience. State-mutating (writes/removes manifests under
        // an agent's workspace). Operational because the ability IS
        // the work, in the same class as ability.deploy /
        // skill.install.
        | "ability.publish"
        | "ability.unpublish"
        | "meta.teach"
        | "meta.acquire"
        | "meta.forget"
        | "skill.publish"
        | "skill.unpublish"
        | "skill.write_file"
        // AXIOM §"Tier 2.5" Baseline Locomotion Profile,
        // filesystem half. fs.read is technically read-only
        // but it returns business content, not just metadata
        // — Operational rather than Observation. fs.write
        // mutates state. fs.list returns directory metadata
        // but its purpose is to enable subsequent fs.read /
        // fs.write — Operational by intent.
        | "fs.read"
        | "fs.write"
        | "fs.stat"
        | "fs.list"
        | "fs.edit"
        // AXIOM Tier 2.5 execution members. process.exec
        // and shell.run are unconditionally Operational —
        // they spawn processes that may do anything; even
        // with the 8-stage shellguard pipeline gating
        // shell.run dispatch, the layer classification
        // tracks privilege not invocation safety.
        | "process.exec"
        | "shell.run"
        | "http.request"
        | "fs.transfer"
        // RFC-005 v3.2 A1–A8 — physical-channel media verbs.
        // Operational by intent: each one drives an external
        // device (mic / camera / speaker / screen) or remote
        // model (voice / asr). Subject = resource_ura.
        | "mic.subscribe"
        | "camera.subscribe"
        | "camera.snapshot"
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
        | "openai.chat_completions"
        | "openai.list_models"
        // RFC-012 §RemoteWebSurface — browser.* family.
        // Operational by intent: opening a WebView session,
        // streaming frames, injecting input, closing the
        // session all drive an external surface (the user's
        // system WebView) under the caller's identity. Same
        // class as media/* verbs.
        | "browser.open_session"
        | "browser.attach_session"
        | "browser.send_input"
        | "browser.capture_viewport"
        | "browser.close_session"
        // Plugin lifecycle reload mutates the daemon's dynamic
        // ability registration table after an install/update/remove
        // transaction has already committed on disk.
        | "plugin.reload"
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
