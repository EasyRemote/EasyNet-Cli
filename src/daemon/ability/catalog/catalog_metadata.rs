// EasyNet CLI — published-ability catalogue metadata
// ==================================================
//
// The read-only descriptor surface: published names/metadata,
// descriptor paths, descriptions, input schemas, RFC-006 rows.
// Catalog metadata owner for daemon-owned system abilities.

use std::collections::BTreeMap;

use super::{build_registry, build_system_registry, daemon_invocation_contracts};

#[cfg(feature = "axon-pb")]
use crate::daemon::ability::builtins::governance::invocation_cancel as invocation_cancel_ability;
use crate::daemon::ability::builtins::{
    agents::{
        authoring as agent_authoring_ability, chat_history as chat_history_ability,
        discover as discover_ability, lifecycle as agent_lifecycle_ability,
        list as agent_list_ability,
    },
    automation::{
        discuss as discuss_ability, loop_ability, mission as mission_ability,
        orchestration as orchestration_ability, schedule as schedule_ability,
        think as think_ability,
    },
    device_control::{
        ability_management::{ops as device_ops_ability, publish as ability_publish_ability},
        file_edit as fs_edit_ability, file_transfer as file_transfer_ability, files as fs_ability,
        http as http_request_ability, net_tunnel as net_tunnel_ability,
        process as process_exec_ability, session as session_ability, shell as shell_run_ability,
        terminal::{
            attach as terminal_attach_ability, io as terminal_io_ability,
            lifecycle as terminal_lifecycle_ability,
        },
    },
    governance::{
        access_control as access_control_ability, admin_status as admin_status_ability,
        consent as consent_ability, health as ping,
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
        files_store as files_store_ability, list as list_resources_ability, media,
        pages as pages_ability, refresh_remote_targets as refresh_remote_targets_ability,
        skills::{install as skill_install_ability, publish as skill_publish_ability},
        voice as voice_call_ability, watch_remote_targets as watch_remote_targets_ability,
    },
};
use crate::daemon::ability::catalog::system_ability_descriptor_path;
use crate::daemon::ability::descriptors::AbilityHints;
use crate::daemon::ability::dispatch::AxonAbilityCatalog;
use crate::daemon::ability::manifest::AbilityBidiWireKind;
use crate::daemon::ability::names::{
    agents as agent_names, automation as automation_names, device_control as device_names,
    federation as federation_names, governance as governance_names,
    integrations as integration_names, resources as resource_names,
};
use crate::daemon::ability::CallMode as DescriptorCallMode;

/// Descriptor-generation inventory entry.
///
/// Operational entries originate from the deterministic live registry.
/// Contract-only entries originate from capability-state evidence. This type
/// is intentionally separate from live publication so Seam/Unsupported
/// contracts can retain generated TOMLs without becoming callable rows.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemAbilityContract {
    pub name: String,
    pub descriptor_version: String,
    pub description: String,
    pub exposure: crate::daemon::ability::manifest::AbilityExposure,
    pub dedicated_surface: crate::daemon::ability::manifest::AbilityDedicatedSurface,
    pub subject_contract_kind: crate::daemon::ability::manifest::AbilitySubjectContractKind,
    pub subject_contract_ura: Option<String>,
    pub bidi_wire_kind: Option<AbilityBidiWireKind>,
    pub input_schema: serde_json::Value,
    pub output_receipt_schema: serde_json::Value,
    pub call_mode: DescriptorCallMode,
    pub admission_action: crate::daemon::ability::descriptors::AdmissionAction,
    pub receipt_semantics: crate::daemon::ability::descriptors::ReceiptSemantics,
    pub visibility: crate::daemon::ability::descriptors::Visibility,
    pub scope_subjects: crate::daemon::ability::descriptors::ScopeRule,
    pub scope_agents: crate::daemon::ability::descriptors::ScopeRule,
    pub denied_agents: Vec<String>,
    pub hints: AbilityHints,
    pub capability_state: crate::daemon::ability::conformance::CapabilityState,
}

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
    !matches!(
        name,
        discover_ability::DEVICE_DISCOVER_ABILITY
            | plugin_lifecycle_ability::COMPANION_STATUS_ABILITY
            | plugin_lifecycle_ability::COMPANION_RECONCILE_ABILITY
    )
}

/// Local runtime routeability filter for daemon-owned ability handlers.
///
/// This is deliberately narrower/different than `is_publishable_catalog_name`:
/// publishability answers "may this row leave the daemon as public catalogue
/// metadata?", while local routeability answers "may this daemon dispatch the
/// registered handler through the Invocation surface?". `agent.discover` is the
/// local aggregate-discovery front door, so it is routable locally but must not
/// be federated as a public ability row.
pub fn is_local_runtime_routable_catalog_name(name: &str) -> bool {
    name == discover_ability::DEVICE_DISCOVER_ABILITY || is_publishable_catalog_name(name)
}

/// Every published system ability descriptor, in deterministic
/// order `published_ability_names()` returns.
///
/// Dynamic hosted-Agent rows are excluded from this deterministic metadata
/// helper. Daemon publication captures them from the live control plane.
pub fn published_abilities() -> Vec<crate::daemon::ability::descriptors::AbilityDescriptor> {
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
pub fn published_system_abilities() -> Vec<crate::daemon::ability::descriptors::AbilityDescriptor> {
    let registry = build_system_registry();
    published_abilities_from_registry(&registry)
}

/// Every daemon-owned static descriptor contract, including non-operational
/// provider seams that must remain on disk but absent from live publication.
pub fn system_ability_contract_inventory() -> Vec<SystemAbilityContract> {
    system_ability_contract_inventory_for_voice_assembly(
        crate::daemon::ability::conformance::VoiceAssemblyEvidence::default(),
    )
}

pub fn system_ability_contract_inventory_for_voice_assembly(
    voice_assembly: crate::daemon::ability::conformance::VoiceAssemblyEvidence,
) -> Vec<SystemAbilityContract> {
    use crate::daemon::ability::conformance::CapabilityState;

    let mut contracts = BTreeMap::new();
    let voice_contracts = voice_ability_contract_inventory(voice_assembly)
        .into_iter()
        .map(|contract| (contract.name.clone(), contract))
        .collect::<BTreeMap<_, _>>();
    for path in super::iter_system_ability_descriptor_paths() {
        let body = std::fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("read system ability contract {}: {error}", path.display())
        });
        let contract =
            super::ability_toml::parse_ability_contract_toml(&body).unwrap_or_else(|error| {
                panic!("parse system ability contract {}: {error}", path.display())
            });
        let expected_path = super::try_system_ability_descriptor_path(&contract.name)
            .unwrap_or_else(|error| {
                panic!(
                    "resolve system ability contract path for {:?} from {}: {error}",
                    contract.name,
                    path.display()
                )
            });
        assert_eq!(
            path,
            expected_path,
            "system ability contract {:?} is stored at {}, expected {}",
            contract.name,
            path.display(),
            expected_path.display()
        );
        // Voice capability state is an assembly fact. Its canonical
        // contract is parsed from this same TOML and projected once by
        // `voice_ability_contract_inventory`; do not collide that
        // projection with the file's unassembled Seam baseline.
        if voice_contracts.contains_key(&contract.name) {
            continue;
        }
        insert_descriptor_contract(&mut contracts, with_canonical_hints(contract));
    }
    for descriptor in published_system_abilities() {
        let name = descriptor.name.clone();
        let input_schema = descriptor.input_schema().clone();
        let call_mode = descriptor.call_mode();
        let contract = SystemAbilityContract {
            name: name.clone(),
            descriptor_version: descriptor.version.clone(),
            description: descriptor.description.clone(),
            exposure: descriptor
                .metadata
                .get("exposure")
                .and_then(|value| parse_descriptor_exposure(value))
                .unwrap_or(crate::daemon::ability::manifest::AbilityExposure::Internal),
            dedicated_surface: descriptor
                .metadata
                .get("dedicated_surface")
                .and_then(|value| parse_descriptor_dedicated_surface(value))
                .unwrap_or(crate::daemon::ability::manifest::AbilityDedicatedSurface::None),
            subject_contract_kind: descriptor
                .metadata
                .get("subject_contract_kind")
                .and_then(|value| parse_descriptor_subject_contract_kind(value))
                .unwrap_or(
                    crate::daemon::ability::manifest::AbilitySubjectContractKind::RouteTarget,
                ),
            subject_contract_ura: descriptor.metadata.get("subject_contract_ura").cloned(),
            bidi_wire_kind: descriptor
                .metadata
                .get("bidi_wire_kind")
                .and_then(|value| parse_descriptor_bidi_wire_kind(value)),
            input_schema,
            output_receipt_schema: match descriptor.output_receipt_schema() {
                serde_json::Value::Null => serde_json::json!({}),
                schema => schema.clone(),
            },
            call_mode,
            admission_action: descriptor.admission_action(),
            receipt_semantics: descriptor.receipt_semantics().clone(),
            visibility: descriptor.visibility,
            scope_subjects: descriptor.scope_subjects.clone(),
            scope_agents: descriptor.scope_agents.clone(),
            denied_agents: descriptor.denied_agents().to_vec(),
            hints: descriptor.hints.clone(),
            capability_state: CapabilityState::CutoverReady,
        };
        upsert_operational_contract(&mut contracts, contract);
    }

    for contract in voice_contracts.into_values() {
        insert_descriptor_contract(&mut contracts, with_canonical_hints(contract));
    }

    contracts.into_values().collect()
}

fn parse_descriptor_exposure(
    value: &str,
) -> Option<crate::daemon::ability::manifest::AbilityExposure> {
    serde_json::from_value(serde_json::Value::String(value.to_string())).ok()
}

fn parse_descriptor_dedicated_surface(
    value: &str,
) -> Option<crate::daemon::ability::manifest::AbilityDedicatedSurface> {
    serde_json::from_value(serde_json::Value::String(value.to_string())).ok()
}

fn parse_descriptor_subject_contract_kind(
    value: &str,
) -> Option<crate::daemon::ability::manifest::AbilitySubjectContractKind> {
    serde_json::from_value(serde_json::Value::String(value.to_string())).ok()
}

fn parse_descriptor_bidi_wire_kind(value: &str) -> Option<AbilityBidiWireKind> {
    serde_json::from_value(serde_json::Value::String(value.to_string())).ok()
}

/// Voice static contracts derived only from typed capability-state evidence.
/// This path deliberately does not construct the operational registry, so
/// conformance gates cannot accidentally require unrelated runtime services.
pub fn voice_ability_contract_inventory(
    voice_assembly: crate::daemon::ability::conformance::VoiceAssemblyEvidence,
) -> Vec<SystemAbilityContract> {
    use crate::daemon::ability::conformance::voice_capability_state_evidence;

    voice_capability_state_evidence(voice_assembly)
        .into_iter()
        .map(|evidence| {
            let path = system_ability_descriptor_path(evidence.name);
            let body = std::fs::read_to_string(&path).unwrap_or_else(|error| {
                panic!("read canonical Voice contract {}: {error}", path.display())
            });
            let mut contract = super::ability_toml::parse_ability_contract_toml(&body)
                .unwrap_or_else(|error| panic!("parse Voice contract {}: {error}", path.display()));
            assert_eq!(contract.name, evidence.name);
            assert_eq!(contract.call_mode, evidence.call_mode);
            contract.capability_state = evidence.state;
            if let Some(schema) =
                crate::daemon::ability::builtins::resources::voice::output_receipt_schema_for(
                    evidence.name,
                )
            {
                contract.output_receipt_schema = schema;
            } else if evidence.state
                != crate::daemon::ability::conformance::CapabilityState::Unsupported
                && contract.output_receipt_schema.is_null()
            {
                panic!(
                    "operational Voice contract {:?} has no receipt schema",
                    evidence.name
                );
            }
            contract
        })
        .collect()
}

fn insert_descriptor_contract(
    contracts: &mut BTreeMap<String, SystemAbilityContract>,
    contract: SystemAbilityContract,
) {
    use std::collections::btree_map::Entry;
    match contracts.entry(contract.name.clone()) {
        Entry::Vacant(entry) => {
            entry.insert(contract);
        }
        Entry::Occupied(entry) => assert_eq!(
            entry.get(),
            &contract,
            "authority-scoped rows disagree on canonical descriptor contract {:?}",
            contract.name
        ),
    }
}

fn with_canonical_hints(mut contract: SystemAbilityContract) -> SystemAbilityContract {
    // Contract-only descriptors are parsed from their TOML so unsupported/seam
    // surfaces can remain on disk without becoming live runtime rows. Hints are
    // not a second TOML-owned truth, though: they are UI/admission policy
    // projection derived from the same semantic classifier used by live
    // registration. Normalizing here keeps generated static descriptor TOMLs,
    // metadata-only catalogues, and runtime control-plane rows aligned.
    contract.hints = registration_hints("", &contract.name);
    contract
}

fn upsert_operational_contract(
    contracts: &mut BTreeMap<String, SystemAbilityContract>,
    contract: SystemAbilityContract,
) {
    contracts.insert(contract.name.clone(), contract);
}

/// Published system abilities whose authority/projection class was declared as
/// `owner` in the registry.
///
/// This is the descriptor-generation path for implementation profiles.
/// Projection membership comes from `AxonAbilityCatalog::lookup_owner`, not
/// from ability name prefixes. That keeps the profile catalogue aligned with
/// the handler registration truth table and prevents broad namespaces such as
/// `device.*` from accidentally stealing abilities advertised by the remaining
/// direct Device-owner projection or any hosted sub-profile Agent.
pub fn published_system_abilities_for_owner(
    owner: crate::daemon::ability::dispatch::OwnerKind,
) -> Vec<crate::daemon::ability::descriptors::AbilityDescriptor> {
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

/// Unique device-sponsored SystemAgent owner for one public system ability.
///
/// Public names can legitimately exist on more than one owner plane (for
/// example, runtime introspection is published by both a realm Authority and a
/// device-sponsored SystemAgent). Routing to a Device therefore cannot use the
/// name-only `control_plane_owner` lookup, which is intentionally ambiguous in
/// that case. This projection reads every committed control-plane row, filters
/// to SystemAgent owners, de-duplicates call modes, and fails closed unless one
/// SystemAgent owner remains.
pub(crate) fn unique_system_agent_owner_for_public_ability(
    public_ability: &str,
) -> Option<crate::daemon::ability::dispatch::OwnerKind> {
    let public_ability = public_ability.trim();
    if public_ability.is_empty() {
        return None;
    }
    let registry = build_system_registry();
    let mut owners = Vec::new();
    for row in registry.authority_ability_catalog_snapshot() {
        if row.descriptor.name != public_ability
            || !matches!(
                &row.owner,
                crate::daemon::ability::dispatch::OwnerKind::SystemAgent(_)
            )
        {
            continue;
        }
        if !owners.contains(&row.owner) {
            owners.push(row.owner);
        }
    }
    match owners.as_slice() {
        [owner] => Some(owner.clone()),
        _ => None,
    }
}

fn published_abilities_from_registry(
    registry: &AxonAbilityCatalog,
) -> Vec<crate::daemon::ability::descriptors::AbilityDescriptor> {
    published_abilities_from_registry_for_owner(registry, None)
}

fn published_abilities_from_registry_for_owner(
    registry: &AxonAbilityCatalog,
    owner: Option<&crate::daemon::ability::dispatch::OwnerKind>,
) -> Vec<crate::daemon::ability::descriptors::AbilityDescriptor> {
    let contract_only_names =
        crate::daemon::ability::conformance::HubBaseline::required_abilities()
            .iter()
            .filter(|ability| {
                ability.surface
                    != crate::daemon::ability::conformance::BaselineSurface::LocalRegistry
            })
            .map(|ability| ability.name)
            .collect::<std::collections::BTreeSet<_>>();
    registry
        .authority_ability_catalog_snapshot()
        .into_iter()
        .filter(|row| is_publishable_catalog_name(&row.name))
        .filter(|row| !contract_only_names.contains(row.name.as_str()))
        .filter(|row| owner.map(|expected| &row.owner == expected).unwrap_or(true))
        .filter(|row| !row.name.ends_with(".chat"))
        .map(|row| row.descriptor)
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

#[cfg(test)]
pub(crate) fn discovery_hints_for(registry: &AxonAbilityCatalog, name: &str) -> AbilityHints {
    registry
        .canonical_descriptor_for_ability(name)
        .ok()
        .flatten()
        .map(|descriptor| descriptor.hints)
        .unwrap_or_default()
}

/// Normalize semantic-layer purity and one exact registered transport into
/// descriptor hints at the registration boundary.
///
/// This function is pure and does not build or inspect a registry. Static and
/// dynamic registration can therefore attach hints before committing the
/// canonical descriptor without creating a catalogue-read recursion.
pub(crate) fn registration_hints(owner_ura: &str, registry_name: &str) -> AbilityHints {
    // Derive the purity hints from the ability's semantic layer — one
    // source of truth (classify_ability). Introspection/Observation are
    // pure reads (read_only + idempotent: re-issuing yields the same
    // snapshot, no side effect). Control is a pure decision (idempotent,
    // not a business read). Operational verbs change the world (neither).
    // These hints ride meta.list_abilities into the catalog, so the
    // frontend coalesces pure-read invokes from the catalog instead of
    // re-classifying ability names locally. Destructiveness is named only for
    // operations whose public contract explicitly authorizes irreversible
    // deletion; it is never inferred from the broad Operational layer.
    let public_name = crate::core::ura::descriptor_public_ability_name(owner_ura, registry_name);
    let (read_only, idempotent) = match classify_ability(&public_name) {
        Some(AbilityLayer::Introspection) | Some(AbilityLayer::Observation) => (true, true),
        Some(AbilityLayer::Control) => (false, true),
        Some(AbilityLayer::Operational) | None => (false, false),
    };
    AbilityHints {
        read_only,
        destructive: is_destructive_public_ability(&public_name),
        idempotent,
    }
}

/// Public-contract destructive risk.
///
/// This is deliberately separate from `classify_ability`: the semantic layer
/// answers purity/coalescing ("does this invoke mutate state?"), while this
/// policy answers UI and consent risk ("does this invoke remove, revoke, or
/// purge durable authority/data?"). Most Operational verbs are not
/// destructive: `ability.deploy`, `ability.publish`, `terminal.input`, and
/// `mission.run` all mutate state, but their contract is not deletion.
pub(crate) fn is_destructive_public_ability(public_name: &str) -> bool {
    matches!(
        public_name,
        // Principal / trust material removal.
        governance_names::PRINCIPAL_DELETE
            | governance_names::PRINCIPAL_REVOKE_KEY
            | governance_names::PRINCIPAL_REVOKE_ENROLLMENT
            | governance_names::PRINCIPAL_REVOKE_GRANT
            | governance_names::AUTHORITY_BINDING_REVOKE
            | federation_names::IDENTITY_REVOKE_USER_PUBKEY
            | federation_names::REVOKE
            // Device, agent, and package lifecycle deletion.
            | device_names::NODE_REMOVE
            | federation_names::ABILITY_UNINSTALL
            | federation_names::ABILITY_UNPUBLISH
            | agent_names::AGENT_PURGE
            | resource_names::SKILL_REMOVE
            | resource_names::SKILL_UNPUBLISH
    )
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
    if let Some(description) = daemon_invocation_contracts::description_for(name) {
        return description;
    }
    if let Some(description) = keyring_management_description_for(name) {
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
        governance_names::CONSENT_SUBSCRIBE => consent_ability::subscribe_description(),
        governance_names::CONSENT_DECIDE => consent_ability::decide_description(),
        governance_names::CONSENT_LIST_PENDING => consent_ability::list_pending_description(),
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
        device_names::NET_TUNNEL => net_tunnel_ability::description(),
        governance_names::INVOCATION_HISTORY_LIST => {
            invocation_history_ability::list_history_description()
        }
        governance_names::INVOCATION_HISTORY_GET => {
            invocation_history_ability::get_history_description()
        }
        governance_names::INVOCATION_RECORD_GET => {
            invocation_history_ability::get_record_description()
        }
        governance_names::INVOCATION_TRACE_GET => {
            invocation_history_ability::get_trace_description()
        }
        governance_names::INVOCATION_HISTORY_PATH => {
            invocation_history_ability::get_path_description()
        }
        #[cfg(feature = "axon-pb")]
        governance_names::INVOCATION_CANCEL => invocation_cancel_ability::description(),
        governance_names::AUTHORITY_BINDING_GRANT
        | governance_names::AUTHORITY_BINDING_REVOKE
        | governance_names::AUTHORITY_BINDING_LIST
        | governance_names::AUTHORITY_BINDING_CHECK
        | governance_names::POLICY_REQUEST_CREATE
        | governance_names::POLICY_REQUEST_RESOLVE
        | governance_names::POLICY_REQUEST_LIST
        | governance_names::ADMISSION_EXPLAIN => access_control_ability::description_for(name),
        device_names::TERMINAL_CREATE => terminal_lifecycle_ability::description_create(),
        device_names::TERMINAL_LIST => terminal_lifecycle_ability::description_list(),
        device_names::TERMINAL_CLOSE => terminal_lifecycle_ability::description_close(),
        device_names::TERMINAL_ATTACH => terminal_attach_ability::description(),
        device_names::TERMINAL_INPUT => terminal_io_ability::input_description(),
        device_names::TERMINAL_READ => terminal_io_ability::read_description(),
        device_names::TERMINAL_RESIZE => terminal_io_ability::resize_description(),
        device_names::FS_TRANSFER => file_transfer_ability::description(),
        agent_names::AGENT_START => agent_lifecycle_ability::start_agent_description(),
        agent_names::AGENT_STOP => agent_lifecycle_ability::stop_agent_description(),
        agent_names::AGENT_PURGE => agent_lifecycle_ability::purge_agent_description(),
        agent_names::AGENT_PURGE_RECONCILE => {
            agent_lifecycle_ability::purge_reconcile_description()
        }
        agent_names::AGENT_REFRESH => agent_lifecycle_ability::refresh_agents_description(),
        agent_authoring_ability::ABILITY_PUT_AGENT_ABILITY => agent_authoring_ability::DESCRIPTION,
        device_names::NODE_DESCRIBE => device_ops_ability::describe_node_description(),
        device_names::NODE_REMOVE => device_ops_ability::remove_node_description(),
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
        n if files_store_ability::description_for(n).is_some() => {
            files_store_ability::description_for(n).unwrap()
        }
        n if pages_ability::management_ability_specs()
            .iter()
            .any(|spec| spec.relative_name == n) =>
        {
            pages_ability::management_ability_specs()
                .into_iter()
                .find(|spec| spec.relative_name == n)
                .expect("pages spec checked above")
                .description
        }
        refresh_remote_targets_ability::ABILITY_RESOURCE_REFRESH_REMOTE_TARGETS => {
            refresh_remote_targets_ability::description()
        }
        watch_remote_targets_ability::ABILITY_RESOURCE_WATCH_REMOTE_TARGETS => {
            watch_remote_targets_ability::description()
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

fn keyring_management_description_for(name: &str) -> Option<&'static str> {
    Some(match name {
        "device.keyring.create" => {
            "Create a managed Ed25519 signing key in the local keyring-management SystemAgent vault and return its public projection."
        }
        "device.keyring.list" => {
            "List managed signing keys from the local keyring-management SystemAgent vault, optionally filtered by purpose or status."
        }
        "device.keyring.get_public" => {
            "Return the public key and fingerprint for one managed signing key without exposing private key material."
        }
        "device.keyring.rotate" => {
            "Retire an active managed signing key and mint its next epoch successor in the keyring vault."
        }
        "device.keyring.revoke" => {
            "Revoke a managed signing key so it cannot be used for subsequent signing operations."
        }
        "device.keyring.expire_set" => {
            "Set or update the expiry timestamp for a managed signing key."
        }
        "device.keyring.bind_subject" => {
            "Bind a managed signing key to the canonical subject URA it is allowed to sign for."
        }
        "device.keyring.peer_add" => {
            "Record a peer public key by trust-on-first-use metadata for later key resolution."
        }
        "device.keyring.peer_list" => {
            "List peer public-key records known to the local keyring-management SystemAgent."
        }
        "device.keyring.federate_user_identity_token" => {
            "Issue a bounded cross-realm user identity token signed by a managed key bound to the source user URA."
        }
        _ => return None,
    })
}

pub fn try_description_for_owned(name: &str) -> anyhow::Result<String> {
    if name == plugin_lifecycle_ability::COMPANION_STATUS_ABILITY {
        return Ok(plugin_lifecycle_ability::companion_status_description().to_string());
    }
    if name == plugin_lifecycle_ability::COMPANION_RECONCILE_ABILITY {
        return Ok(plugin_lifecycle_ability::companion_reconcile_description().to_string());
    }
    if super::try_system_ability_descriptor_path(name).is_ok_and(|path| path.is_file()) {
        return Ok(super::system_manifest::canonical_registration_contract(name)?.description);
    }
    if let Some(description) = crate::daemon::plugins::try_builtin_description_for_owned(name)? {
        return Ok(description);
    }
    if let Some(description) = crate::daemon::plugins::try_description_for_owned(name)? {
        return Ok(description);
    }
    Ok(description_for(name).to_string())
}

/// JSON Schema for a published system ability's input. Mirrors
/// `description_for` — adding an arm here is the second half of
/// landing a new system ability so it can register against
/// axon-runtime with an authored schema.
///
/// Undeclared names project through `CatalogSchemaProjection::UndeclaredObject`:
/// a valid object JSON Schema that preserves the internal distinction between
/// authored no-arg schemas and missing metadata. CI pins the live registry so
/// published system abilities cannot accidentally ship in that state.
pub fn try_input_schema_for(name: &str) -> anyhow::Result<serde_json::Value> {
    Ok(CatalogSchemaProjection::try_for_input_name(name)?.into_schema())
}

#[derive(Debug, Clone, PartialEq)]
enum CatalogSchemaProjection {
    Declared(serde_json::Value),
    UndeclaredObject,
}

impl CatalogSchemaProjection {
    fn try_for_input_name(name: &str) -> anyhow::Result<Self> {
        Ok(match Self::try_declared_input_schema(name)? {
            Some(schema) => Self::Declared(schema),
            None => Self::UndeclaredObject,
        })
    }

    fn try_declared_input_schema(name: &str) -> anyhow::Result<Option<serde_json::Value>> {
        if matches!(
            name,
            plugin_lifecycle_ability::COMPANION_STATUS_ABILITY
                | plugin_lifecycle_ability::COMPANION_RECONCILE_ABILITY
        ) {
            return Ok(Some(plugin_lifecycle_ability::companion_input_schema()));
        }
        if super::try_system_ability_descriptor_path(name).is_ok_and(|path| path.is_file()) {
            return Ok(Some(
                super::system_manifest::canonical_registration_contract(name)?.input_schema,
            ));
        }
        if let Some(schema) = crate::daemon::plugins::try_builtin_input_schema_for(name)? {
            return Ok(Some(schema));
        }
        if let Some(schema) = crate::daemon::plugins::try_input_schema_for(name)? {
            return Ok(Some(schema));
        }
        if let Some(schema) = daemon_invocation_contracts::input_schema_for(name) {
            return Ok(Some(schema));
        }
        Ok(authored_static_input_schema(name))
    }

    fn into_schema(self) -> serde_json::Value {
        match self {
            Self::Declared(schema) => schema,
            Self::UndeclaredObject => serde_json::json!({ "type": "object" }),
        }
    }
}

fn authored_static_input_schema(name: &str) -> Option<serde_json::Value> {
    Some(match name {
        governance_names::OBSERVE_HEALTH => ping::input_schema(),
        governance_names::OBSERVE_NETWORK_HEALTH => network_health_ability::input_schema(),
        device_names::SESSION_LIST => session_ability::list_input_schema(),
        device_names::SESSION_ATTACH => session_ability::attach_input_schema(),
        agent_names::CHAT_HISTORY_LIST => chat_history_ability::list_input_schema(),
        agent_names::CHAT_HISTORY_GET => chat_history_ability::get_input_schema(),
        name if name.starts_with("context.") => return context_ability::input_schema_for(name),
        governance_names::CONSENT_SUBSCRIBE => consent_ability::subscribe_input_schema(),
        governance_names::CONSENT_DECIDE => consent_ability::decide_input_schema(),
        governance_names::CONSENT_LIST_PENDING => consent_ability::list_pending_input_schema(),
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
        name if governance_names::KEYRING_ABILITIES.contains(&name) => {
            return crate::daemon::keyring::abilities::input_schema_for(name)
        }
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
        device_names::NET_TUNNEL => net_tunnel_ability::input_schema(),
        governance_names::INVOCATION_HISTORY_LIST => {
            invocation_history_ability::list_history_input_schema()
        }
        governance_names::INVOCATION_HISTORY_GET => {
            invocation_history_ability::get_history_input_schema()
        }
        governance_names::INVOCATION_RECORD_GET => {
            invocation_history_ability::get_record_input_schema()
        }
        governance_names::INVOCATION_TRACE_GET => {
            invocation_history_ability::get_trace_input_schema()
        }
        governance_names::INVOCATION_HISTORY_PATH => {
            invocation_history_ability::get_path_input_schema()
        }
        #[cfg(feature = "axon-pb")]
        governance_names::INVOCATION_CANCEL => invocation_cancel_ability::input_schema(),
        governance_names::AUTHORITY_BINDING_GRANT
        | governance_names::AUTHORITY_BINDING_REVOKE
        | governance_names::AUTHORITY_BINDING_LIST
        | governance_names::AUTHORITY_BINDING_CHECK
        | governance_names::POLICY_REQUEST_CREATE
        | governance_names::POLICY_REQUEST_RESOLVE
        | governance_names::POLICY_REQUEST_LIST
        | governance_names::ADMISSION_EXPLAIN => access_control_ability::input_schema_for(name),
        device_names::TERMINAL_CREATE => terminal_lifecycle_ability::input_schema_create(),
        device_names::TERMINAL_LIST => terminal_lifecycle_ability::input_schema_list(),
        device_names::TERMINAL_CLOSE => terminal_lifecycle_ability::input_schema_close(),
        device_names::TERMINAL_ATTACH => terminal_attach_ability::input_schema(),
        device_names::TERMINAL_INPUT => terminal_io_ability::input_input_schema(),
        device_names::TERMINAL_READ => terminal_io_ability::read_input_schema(),
        device_names::TERMINAL_RESIZE => terminal_io_ability::resize_input_schema(),
        device_names::FS_TRANSFER => file_transfer_ability::input_schema(),
        agent_names::AGENT_START => agent_lifecycle_ability::start_agent_input_schema(),
        agent_names::AGENT_STOP => agent_lifecycle_ability::stop_agent_input_schema(),
        agent_names::AGENT_PURGE => agent_lifecycle_ability::purge_agent_input_schema(),
        agent_names::AGENT_PURGE_RECONCILE => {
            agent_lifecycle_ability::purge_reconcile_input_schema()
        }
        agent_names::AGENT_REFRESH => agent_lifecycle_ability::refresh_agents_input_schema(),
        agent_authoring_ability::ABILITY_PUT_AGENT_ABILITY => {
            agent_authoring_ability::input_schema()
        }
        device_names::NODE_DESCRIBE => device_ops_ability::describe_node_input_schema(),
        device_names::NODE_REMOVE => device_ops_ability::remove_node_input_schema(),
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
        name if media::input_schema(name).is_some() => return media::input_schema(name),
        list_resources_ability::ABILITY_META_LIST_RESOURCES => {
            list_resources_ability::input_schema()
        }
        n if files_store_ability::input_schema_for(n).is_some() => {
            return files_store_ability::input_schema_for(n)
        }
        n if pages_ability::management_ability_specs()
            .iter()
            .any(|spec| spec.relative_name == n) =>
        {
            return Some(
                pages_ability::management_ability_specs()
                    .into_iter()
                    .find(|spec| spec.relative_name == n)
                    .expect("pages spec checked above")
                    .input_schema,
            )
        }
        refresh_remote_targets_ability::ABILITY_RESOURCE_REFRESH_REMOTE_TARGETS => {
            refresh_remote_targets_ability::input_schema()
        }
        watch_remote_targets_ability::ABILITY_RESOURCE_WATCH_REMOTE_TARGETS => {
            watch_remote_targets_ability::input_schema()
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
        _ => return None,
    })
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

    if let Some(layer) = daemon_invocation_contracts::contract_layer(name) {
        return Some(match layer {
            daemon_invocation_contracts::DaemonInvocationContractLayer::Introspection => {
                AbilityLayer::Introspection
            }
            daemon_invocation_contracts::DaemonInvocationContractLayer::Control => {
                AbilityLayer::Control
            }
            daemon_invocation_contracts::DaemonInvocationContractLayer::Operational => {
                AbilityLayer::Operational
            }
        });
    }
    if super::runtime_admin_contracts::contains(name) {
        return Some(AbilityLayer::Operational);
    }

    let canonical_layer = match name {
        // ── Introspection ───────────────────────────────────
        governance_names::META_DESCRIBE
        | governance_names::META_LIST_ABILITIES
        | federation_names::RESOLVE
        | federation_names::DISCOVER
        | federation_names::STATUS
        | federation_names::NAMESPACE_RESOLVE
        | federation_names::RESOLVE_KEY
        | federation_names::IDENTITY_LIST_USER_PUBKEYS
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
        // Daemon-local aggregate discovery front door. It fans in directory
        // snapshots and does not mutate agent or federation state.
        | agent_names::DISCOVER
        | agent_names::AGENT_LIST
        | governance_names::INVOCATION_HISTORY_LIST
        | governance_names::INVOCATION_HISTORY_GET
        | governance_names::INVOCATION_RECORD_GET
        | governance_names::INVOCATION_TRACE_GET
        | governance_names::INVOCATION_HISTORY_PATH
        | governance_names::AUTHORITY_BINDING_LIST
        | governance_names::AUTHORITY_BINDING_CHECK
        | governance_names::POLICY_REQUEST_LIST
        | governance_names::ADMISSION_EXPLAIN
        | device_names::TERMINAL_LIST
        | device_names::SESSION_LIST
        | "device.keyring.list"
        | "device.keyring.get_public"
        | "device.keyring.peer_list"
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
        | "files.get"
        | "files.list"
        | "project_list"
        | "pages.get"
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
        | "context.catalog"
        | "context.folders.list"
        | "context.fs.list"
        | "context.favorites.list"
        | "context.captures.list"
        | "context.captures.get"
        | "context.captures.read"
        | resource_names::VOICE_SHOW_CALL
        | resource_names::VOICE_WATCH_CALL
        | resource_names::VOICE_LIST_CALLS => Some(AbilityLayer::Introspection),
        // ── Control / decision ──────────────────────────────
        governance_names::CONSENT_DECIDE
        | governance_names::INVOCATION_CANCEL
        | agent_names::AGENT_PURGE_RECONCILE
        | governance_names::AUTHORITY_BINDING_GRANT
        | governance_names::AUTHORITY_BINDING_REVOKE
        | governance_names::POLICY_REQUEST_CREATE
        | governance_names::POLICY_REQUEST_RESOLVE
        | "device.keyring.peer_add"
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
        | "pages.health"
        | "plugin.status"
        | "plugin.companion_status" => Some(AbilityLayer::Observation),
        // ── Operational (per-feature business verbs) ────────
        device_names::SESSION_ATTACH
        | agent_names::AGENT_START
        | agent_names::AGENT_STOP
        | agent_names::AGENT_PURGE
        | agent_names::AGENT_REFRESH
        | agent_authoring_ability::ABILITY_PUT_AGENT_ABILITY
        | resource_names::SKILL_INSTALL
        | resource_names::SKILL_REMOVE
        | resource_names::SKILL_UPGRADE
        // device-hosted node/ability/remote operations. `node.describe` reads
        // the canonical federation-backed device view for one device; fleet
        // enumeration belongs to `federation.discover`, not a second
        // device-owned route. The remaining verbs (remove_node, deploy_ability,
        // uninstall_ability) mutate state — Operational unambiguous.
        | device_names::NODE_DESCRIBE
        | device_names::NODE_REMOVE
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
        // ability. Same Operational class as mission.run because
        // the ability IS the work
        // (running one human-bracketed sub-turn of a
        // multi-agent discussion).
        | automation_names::MISSION_DISCUSS_ROUND
        // mission.think — long-running worker+judge loop. Same
        // Operational rationale: the ability IS the work
        // (running an N-cycle reflective loop with two
        // independent chat sessions).
        | automation_names::MISSION_THINK
        // Voice call mutations are operational. Read-only call inspection is
        // classified above so its descriptor receives Read while metrics
        // remains an explicit mutation despite using RPC geometry.
        | resource_names::VOICE_CREATE_CALL
        | resource_names::VOICE_JOIN_CALL
        | resource_names::VOICE_LEAVE_CALL
        | resource_names::VOICE_END_CALL
        | resource_names::VOICE_REPORT_METRICS
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
        | "device.keyring.create"
        | "device.keyring.rotate"
        | "device.keyring.revoke"
        | "device.keyring.expire_set"
        | "device.keyring.bind_subject"
        | "device.keyring.federate_user_identity_token"
        // EAL orchestration. mission.run compiles and executes a
        // program (potentially multi-step, potentially cross-agent);
        // mission.cancel mutates the run state of an in-flight
        // mission. Same Operational class as loop.{create,cancel}
        // for the same reason — the ability IS the work.
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
        | "files.put"
        | "pages.publish"
        | "pages.unpublish"
        | resource_names::RESOURCE_REFRESH_REMOTE_TARGETS
        | resource_names::RESOURCE_WATCH_REMOTE_TARGETS
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
        | device_names::NET_TUNNEL
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
        // Plugin lifecycle reload mutates the daemon's dynamic
        // ability registration table after an install/update/remove
        // transaction has already committed on disk.
        | "plugin.reload"
        | "plugin.activate_realtime"
        | "plugin.companion_reconcile"
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
    };
    if canonical_layer.is_some() {
        return canonical_layer;
    }

    // Plugin state is runtime-owned and may resolve through a user-selected
    // package root. Consult it only after every canonical daemon contract has
    // been classified from static control-plane facts. Otherwise constructing
    // the deterministic system registry merely to answer an ownership query
    // reads `$HOME/.easynet/plugins`, coupling descriptor identity to ambient
    // process state and making pure route projection race with unrelated
    // environment-isolation tests.
    crate::daemon::plugins::ability_layer_for(name).map(|layer| match layer {
        crate::daemon::plugins::PluginAbilityLayer::Introspection => AbilityLayer::Introspection,
        crate::daemon::plugins::PluginAbilityLayer::Control => AbilityLayer::Control,
        crate::daemon::plugins::PluginAbilityLayer::Observation => AbilityLayer::Observation,
        crate::daemon::plugins::PluginAbilityLayer::Operational => AbilityLayer::Operational,
    })
}

#[cfg(test)]
mod canonical_contract_tests {
    use super::*;
    use crate::daemon::ability::conformance::CapabilityState;
    use crate::daemon::ability::descriptors::{
        AdmissionAction, ReceiptSemantics, ScopeRule, StateTransition, TransitionClass, Visibility,
    };

    #[test]
    fn catalog_schema_projection_distinguishes_declared_from_undeclared_object() {
        let declared =
            CatalogSchemaProjection::try_for_input_name(governance_names::CONSENT_SUBSCRIBE)
                .expect("declared schema projection");
        assert!(
            matches!(declared, CatalogSchemaProjection::Declared(_)),
            "authored no-arg schemas must remain declared, not undeclared object projections"
        );
        let declared_schema = declared.into_schema();
        assert_eq!(declared_schema["type"], "object");
        assert_eq!(declared_schema["additionalProperties"], false);

        let undeclared =
            CatalogSchemaProjection::try_for_input_name("runtime.test.unpublished_schema_probe")
                .expect("undeclared schema projection");
        assert_eq!(undeclared, CatalogSchemaProjection::UndeclaredObject);
        assert_eq!(
            undeclared.into_schema(),
            serde_json::json!({ "type": "object" })
        );
    }

    #[test]
    fn catalog_schema_projection_treats_context_table_hits_as_declared() {
        let projection =
            CatalogSchemaProjection::try_for_input_name(context_ability::ABILITY_CLIPBOARD_LIST)
                .expect("context schema projection");

        assert!(
            matches!(projection, CatalogSchemaProjection::Declared(_)),
            "context.* schemas must pass through declared catalogue projection"
        );
        assert_ne!(
            projection.into_schema(),
            serde_json::json!({ "type": "object" }),
            "declared context schema must not collapse to the undeclared object projection"
        );
    }

    #[test]
    fn authority_rows_reject_every_canonical_contract_difference() {
        let baseline = system_ability_contract_inventory()
            .into_iter()
            .find(|contract| contract.name == "voice.report_metrics")
            .expect("voice report metrics contract");
        let variants: Vec<SystemAbilityContract> = vec![
            SystemAbilityContract {
                descriptor_version: "9.9.9".to_string(),
                ..baseline.clone()
            },
            SystemAbilityContract {
                exposure: crate::daemon::ability::manifest::AbilityExposure::Internal,
                ..baseline.clone()
            },
            SystemAbilityContract {
                dedicated_surface:
                    crate::daemon::ability::manifest::AbilityDedicatedSurface::Terminal,
                ..baseline.clone()
            },
            SystemAbilityContract {
                subject_contract_kind:
                    crate::daemon::ability::manifest::AbilitySubjectContractKind::ExplicitUra,
                ..baseline.clone()
            },
            SystemAbilityContract {
                subject_contract_ura: Some(
                    "easynet:///r/test/resource/device.dev-a/session/test".to_string(),
                ),
                ..baseline.clone()
            },
            SystemAbilityContract {
                input_schema: serde_json::json!({"type":"array"}),
                ..baseline.clone()
            },
            SystemAbilityContract {
                call_mode: DescriptorCallMode::Stream,
                ..baseline.clone()
            },
            SystemAbilityContract {
                admission_action: AdmissionAction::Read,
                ..baseline.clone()
            },
            SystemAbilityContract {
                output_receipt_schema: serde_json::json!({"type":"object"}),
                ..baseline.clone()
            },
            SystemAbilityContract {
                receipt_semantics: ReceiptSemantics::StateTransition(
                    StateTransition::new("voice.report_metrics@v1", TransitionClass::Canonical)
                        .expect("valid test transition"),
                ),
                ..baseline.clone()
            },
            SystemAbilityContract {
                visibility: Visibility::Public,
                ..baseline.clone()
            },
            SystemAbilityContract {
                scope_subjects: ScopeRule::None,
                ..baseline.clone()
            },
            SystemAbilityContract {
                scope_agents: ScopeRule::None,
                ..baseline.clone()
            },
            SystemAbilityContract {
                denied_agents: vec!["easynet:///r/test/agent/denied".to_string()],
                ..baseline.clone()
            },
            SystemAbilityContract {
                hints: AbilityHints {
                    destructive: !baseline.hints.destructive,
                    ..baseline.hints.clone()
                },
                ..baseline.clone()
            },
            SystemAbilityContract {
                capability_state: CapabilityState::Unsupported,
                ..baseline.clone()
            },
        ];

        for variant in variants {
            let mut contracts = BTreeMap::new();
            insert_descriptor_contract(&mut contracts, baseline.clone());
            assert!(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    insert_descriptor_contract(&mut contracts, variant)
                }))
                .is_err(),
                "canonical conflict must fail closed"
            );
        }
    }
}
