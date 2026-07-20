// EasyNet CLI — device node, ability, and remote operation abilities
// ==========================================
//
// File: src/daemon/ability/builtins/device_control/ability_management/ops.rs
// Description: Device-hosted abilities the CLI's
//              device/ability subcommands invoke. Replaces the
//              former direct calls to bridge fns
//              (`list_nodes`, `publish_capability`, etc.) that
//              AXON-RFC-001 P1.5 removed; the ability surface
//              survives unchanged regardless of which transport
//              backs them, in line with the ontology that says
//              "every action is an ability invocation."
//
// Abilities registered here
// -------------------------
//   node.list        List device nodes (this device + known peers).
//   node.describe     Describe one node by id.
//   node.remove       Remove a node from the realm device registry.
//   ability.deploy    Publish an ability bundle to a target node.
//   ability.uninstall Uninstall a previously deployed ability.
//
// Routing model
// -------------
// Every handler accepts `node_id` (or `target_node_id`); the value
// `"local"` (or absent) means "this device" and is fully implemented
// in-process. Any other id is a federation-tier target — the
// transport that fans the call out across the realm was removed by
// AXON-RFC-001 P1.5 and will be re-wired as a federation Invoke
// surface. Until then, those handlers return a typed
// `federation_not_wired` error so callers see the same actionable
// message every CLI surface produces.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::{Arc, OnceLock};

use serde_json::{json, Value};

use crate::daemon::ability::builtins::agents::discover::{
    DiscoverFederationResolver, SharedDiscoverFederationResolver,
};
use crate::daemon::ability::builtins::device_control::ability_management::registrar::{
    DeviceAbilityInstall, DeviceAbilityRegistrar, DeviceAbilityUninstall,
};
#[cfg(test)]
use crate::daemon::ability::builtins::device_control::ability_management::store::DeviceAbilityStore;
use crate::daemon::ability::builtins::device_control::ability_management::store::{
    manifest_digest, DeviceAbilityRecord,
};
use crate::daemon::ability::builtins::integrations::federation_probe;
use crate::daemon::ability::dispatch::{AxonAbilityCatalog, EnvelopeContext, OwnerKind};
use crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA;
use crate::daemon::resources::files::{self as filesystem, FilesystemResourceCapability};
use crate::support::async_bridge::{run_blocking, NoRuntimeFallback};

/// Shared, late-wired cell holding the device-ability registrar.
/// Constructed pending at registry-build time; boot attaches the live
/// `LocalRuntime` via `set_runtime`. `ability.deploy`'s handler reads
/// it to run the install transaction. Mirrors
/// `agent_lifecycle_ability::SharedHotRegistrarCell`.
pub type SharedDeviceRegistrarCell = OnceLock<Arc<DeviceAbilityRegistrar>>;

pub const ABILITY_LIST_NODES: &str = crate::daemon::ability::names::federation::NODE_LIST;
pub const ABILITY_DESCRIBE_NODE: &str = crate::daemon::ability::names::federation::NODE_DESCRIBE;
pub const ABILITY_REMOVE_NODE: &str = crate::daemon::ability::names::federation::NODE_REMOVE;
pub const ABILITY_DEPLOY_ABILITY: &str = crate::daemon::ability::names::federation::ABILITY_DEPLOY;
pub const ABILITY_UNINSTALL_ABILITY: &str =
    crate::daemon::ability::names::federation::ABILITY_UNINSTALL;

const RESERVED_DEVICE_ABILITY_NAMESPACES: &[&str] = &[
    "ability", "device", "hub", "meta", "node", "remote", "system",
];

trait DeviceOpsClock {
    fn now_unix_ms(&self) -> u64;
}

struct SystemDeviceOpsClock;

impl DeviceOpsClock for SystemDeviceOpsClock {
    fn now_unix_ms(&self) -> u64 {
        chrono::Utc::now().timestamp_millis().max(0) as u64
    }
}

/// Register every device operation handler on `reg`. Called once
/// at daemon boot from `daemon::ability::catalog::build_registry_with_services`.
pub fn register(
    reg: &mut AxonAbilityCatalog,
    device_registrar: Arc<SharedDeviceRegistrarCell>,
    local_catalog: Arc<OnceLock<Arc<AxonAbilityCatalog>>>,
    resolver: SharedDiscoverFederationResolver,
) {
    let list_resolver = Arc::clone(&resolver);
    reg.register_rpc_with_spec(
        ABILITY_LIST_NODES,
        OwnerKind::Device,
        crate::daemon::ability::catalog::system_manifest::registry_manifest(
            ABILITY_LIST_NODES,
            list_nodes_description(),
            list_nodes_input_schema(),
        ),
        Arc::new(move |args| list_nodes_handler(args, list_resolver.as_ref())),
    );
    let describe_resolver = Arc::clone(&resolver);
    let describe_catalog = Arc::clone(&local_catalog);
    reg.register_rpc_with_spec(
        ABILITY_DESCRIBE_NODE,
        OwnerKind::Device,
        crate::daemon::ability::catalog::system_manifest::registry_manifest(
            ABILITY_DESCRIBE_NODE,
            describe_node_description(),
            describe_node_input_schema(),
        ),
        Arc::new(move |args| {
            describe_node_handler(args, describe_resolver.as_ref(), &describe_catalog)
        }),
    );
    reg.register_rpc_with_spec(
        ABILITY_REMOVE_NODE,
        OwnerKind::Device,
        crate::daemon::ability::catalog::system_manifest::registry_manifest(
            ABILITY_REMOVE_NODE,
            remove_node_description(),
            remove_node_input_schema(),
        ),
        Arc::new(remove_node_handler),
    );
    reg.register_rpc_with_envelope_and_spec(
        ABILITY_DEPLOY_ABILITY,
        OwnerKind::Device,
        crate::daemon::ability::catalog::system_manifest::registry_manifest(
            ABILITY_DEPLOY_ABILITY,
            deploy_ability_description(),
            deploy_ability_input_schema(),
        ),
        {
            let cell = Arc::clone(&device_registrar);
            Arc::new(move |env: EnvelopeContext, args: Value| {
                deploy_ability_handler(env, args, &cell)
            })
        },
    );
    reg.register_rpc_with_envelope_and_spec(
        ABILITY_UNINSTALL_ABILITY,
        OwnerKind::Device,
        crate::daemon::ability::catalog::system_manifest::registry_manifest(
            ABILITY_UNINSTALL_ABILITY,
            uninstall_ability_description(),
            uninstall_ability_input_schema(),
        ),
        {
            let cell = Arc::clone(&device_registrar);
            Arc::new(move |env: EnvelopeContext, args: Value| {
                uninstall_ability_handler(env, args, &cell)
            })
        },
    );
}

// ── Helpers ──────────────────────────────────────────────────────

/// Resolve the local node's identity from credentials + runtime state.
/// Returns the `(node_id, tenant_id, hub_endpoint, paired)` tuple that
/// every device operation handler needs to know "what is this device". `paired
/// = false` when `~/.easynet/credentials.json` is absent — the
/// daemon may still serve local abilities, but federation-tier
/// answers should reflect the unpaired state.
fn local_identity() -> (String, String, Option<String>, bool) {
    let local = federation_probe::local_identity();
    (
        local.node_id,
        local.tenant_id,
        local.hub_endpoint,
        local.paired,
    )
}

/// Treat a node id as "this device". Accepts the literal `local`,
/// the empty string (omitted flag), and the device's actual node_id
/// from credentials. Any other value is a remote target, deferred
/// to the federation-Invoke replacement.
fn is_local_target(node_id: &str, local_node_id: &str) -> bool {
    let trimmed = node_id.trim();
    trimmed.is_empty() || trimmed == "local" || trimmed == local_node_id
}

fn require_local_device_authority(
    env: &EnvelopeContext,
    expected_device_ura: &str,
    surface: &str,
) -> anyhow::Result<String> {
    let caller = env.caller();
    if caller != expected_device_ura && caller != LOCAL_SYSTEM_AGENT_URA {
        anyhow::bail!(
            "{surface}: caller {caller:?} is not authorized to mutate local device abilities; \
             expected local device authority {expected_device_ura:?}"
        );
    }
    Ok(caller.to_string())
}

/// Surface the canonical "federation not wired" error from an
/// ability handler. The string mirrors `support::local_invoke`'s
/// helper byte-for-byte so a CLI script that greps the message sees
/// the same wording whether the error came from CLI-side validation
/// (e.g. `--node bogus`) or daemon-side dispatch (here).
fn federation_not_wired(action: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "{action} requires the federation Invoke surface, which was removed by \
         AXON-RFC-001 P1.5 and has not yet been re-published as a \
         federation-tier ability. Local-only operations remain available — \
         see `easynet ability list` for what this node can do without \
         federation. The replacement (Invoke against an Agent ability on \
         the realm) ships in a follow-up; this command will be re-wired \
         without changing its CLI shape when it lands."
    )
}

// ── node.list ─────────────────────────────────────────────

/// List every node visible from this device. v1: just the local
/// node (federation peer enumeration depends on the dead bridge
/// `list_nodes`; will be re-wired through a federation Invoke
/// helper when one ships, at which point this handler fan-outs).
fn list_nodes_handler(
    _args: Value,
    resolver: &dyn DiscoverFederationResolver,
) -> anyhow::Result<Value> {
    let view = federation_probe::collect_device_view(resolver);
    let nodes: Vec<Value> = view
        .nodes
        .iter()
        .map(federation_probe::node_to_json)
        .collect();
    Ok(json!({
        "nodes": nodes,
        "federation_view": view.federation_view,
        "federation_view_reason": view.federation_view_reason,
        "resolve_latency_ms": view.resolve_latency_ms,
    }))
}

// ── node.describe ──────────────────────────────────────────

fn describe_node_handler(
    args: Value,
    resolver: &dyn DiscoverFederationResolver,
    local_catalog: &OnceLock<Arc<AxonAbilityCatalog>>,
) -> anyhow::Result<Value> {
    let node_id = args
        .get("node_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if node_id.is_empty() {
        anyhow::bail!("node.describe: `node_id` is required");
    }
    let (local_id, _tenant, _hub, paired) = local_identity();
    if is_local_target(node_id, &local_id) {
        let catalog = local_catalog.get().ok_or_else(|| {
            anyhow::anyhow!("node.describe: live ability catalog is not attached")
        })?;
        if let Some(record) = federation_probe::local_device_record(catalog.as_ref())? {
            return Ok(node_json_with_abilities(
                &record.node,
                record.ability_summaries,
            ));
        }
        if paired {
            if let Some(record) = federation_probe::resolve_device_record(resolver, &local_id)? {
                return Ok(node_json_with_abilities(
                    &record.node,
                    record.ability_summaries,
                ));
            }
        }
        let view = federation_probe::collect_device_view(resolver);
        let node = view
            .nodes
            .iter()
            .find(|n| n.is_self)
            .ok_or_else(|| anyhow::anyhow!("node.describe: local node is unavailable"))?;
        return Ok(federation_probe::node_to_json(node));
    }
    if let Some(record) = federation_probe::resolve_device_record(resolver, node_id)? {
        return Ok(node_json_with_abilities(
            &record.node,
            record.ability_summaries,
        ));
    }

    let view = federation_probe::collect_device_view(resolver);
    let suffix = view
        .federation_view_reason
        .as_deref()
        .map(|reason| format!(" ({reason})"))
        .unwrap_or_default();
    anyhow::bail!("node.describe: node {node_id:?} not found{suffix}");
}

fn node_json_with_abilities(
    node: &federation_probe::DeviceNodeSnapshot,
    abilities: Vec<Value>,
) -> Value {
    let mut value = federation_probe::node_to_json(node);
    if let Value::Object(map) = &mut value {
        map.insert("abilities".to_string(), Value::Array(abilities));
    }
    value
}

// ── node.remove ────────────────────────────────────────────

fn remove_node_handler(args: Value) -> anyhow::Result<Value> {
    let node_id = args
        .get("node_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if node_id.is_empty() {
        anyhow::bail!("node.remove: `node_id` is required");
    }
    let (local_id, _tenant, _hub, _paired) = local_identity();
    if is_local_target(node_id, &local_id) {
        anyhow::bail!(
            "node.remove refuses to remove this device (would delete its own \
             pairing). Use `easynet device reset` for that — it is the local \
             side of the same operation."
        );
    }
    Err(federation_not_wired(&format!(
        "removing the remote node {node_id:?}"
    )))
}

// ── ability.deploy ─────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct AbilityNamespace(String);

impl AbilityNamespace {
    fn parse(raw: Option<&str>) -> anyhow::Result<Self> {
        let raw = raw
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "ability.deploy: ability.json must declare non-empty `namespace`; \
                     deployed device abilities are always registered as `<namespace>.<name>`"
                )
            })?;
        if RESERVED_DEVICE_ABILITY_NAMESPACES.contains(&raw) {
            anyhow::bail!(
                "ability.deploy: namespace {raw:?} is reserved for daemon-owned ability surfaces"
            );
        }
        let mut chars = raw.chars();
        let first = chars
            .next()
            .expect("namespace was checked non-empty before validation");
        if !first.is_ascii_alphabetic() {
            anyhow::bail!("ability.deploy: namespace {raw:?} must start with an ASCII letter");
        }
        if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            anyhow::bail!(
                "ability.deploy: namespace {raw:?} may contain only ASCII letters, digits, `_`, or `-`"
            );
        }
        Ok(Self(raw.to_string()))
    }

    fn wire_key(&self, public_name: &str) -> String {
        format!("{}.{public_name}", self.0)
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

struct AbilityBundle {
    display_path: String,
    /// Absolute path to the bundle's `ability.json` (durable store key).
    manifest_path: String,
    /// Raw manifest bytes (for the durable manifest_hash).
    manifest_bytes: Vec<u8>,
    /// The deserialized manifest. EasyRemote writes extra fields
    /// (`category`, `command`, `tool_name`, …); `AbilityManifest` has no
    /// `deny_unknown_fields`, so they are ignored, and the canonical
    /// `name` / `input_schema` / `exec` come through typed.
    manifest: crate::daemon::ability::manifest::AbilityManifest,
    /// Verb-only local name (`generate`); namespace is separate.
    public_name: String,
    /// Namespace segment (`er`) when the manifest declares one.
    namespace: AbilityNamespace,
}

impl AbilityBundle {
    fn from_resource_ref(args: &Value) -> anyhow::Result<Self> {
        let resolved =
            filesystem::resolve_filesystem_path(args, FilesystemResourceCapability::Read)?;
        let dir = resolved.local_path;
        let display_path = resolved.display_path;
        if !dir.is_dir() {
            anyhow::bail!("ability.deploy: resource_ref {display_path:?} is not a directory");
        }

        let manifest_file = dir.join("ability.json");
        if !manifest_file.is_file() {
            anyhow::bail!(
                "ability.deploy: resource_ref {display_path:?} does not contain an ability.json"
            );
        }

        let manifest_bytes = std::fs::read(&manifest_file)?;
        let manifest =
            crate::daemon::ability::manifest::AbilityManifest::from_json_slice(&manifest_bytes)
                .map_err(|e| {
                    anyhow::anyhow!("invalid ability.json at {display_path}/ability.json: {e}")
                })?;

        // EasyRemote may carry the namespace separately; the manifest
        // `name` is the verb only (AbilityManifest.name forbids dots).
        let namespace = AbilityNamespace::parse(
            serde_json::from_slice::<Value>(&manifest_bytes)
                .ok()
                .and_then(|v| {
                    v.get("namespace")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .as_deref(),
        )?;

        Ok(Self {
            display_path,
            manifest_path: manifest_file.to_string_lossy().into_owned(),
            manifest_bytes,
            public_name: manifest.name().to_string(),
            namespace,
            manifest,
        })
    }

    /// Wire dispatch key: `namespace.verb` when a namespace is present,
    /// else the bare verb. This is the registry key the route resolver
    /// will see (`er.generate`).
    fn wire_key(&self) -> String {
        self.namespace.wire_key(&self.public_name)
    }
}

fn deploy_ability_handler(
    env: EnvelopeContext,
    args: Value,
    device_registrar: &SharedDeviceRegistrarCell,
) -> anyhow::Result<Value> {
    deploy_ability_handler_with_clock(env, args, device_registrar, &SystemDeviceOpsClock)
}

fn deploy_ability_handler_with_clock(
    env: EnvelopeContext,
    args: Value,
    device_registrar: &SharedDeviceRegistrarCell,
    clock: &dyn DeviceOpsClock,
) -> anyhow::Result<Value> {
    let node_id = args
        .get("node_id")
        .and_then(Value::as_str)
        .unwrap_or("local")
        .trim();
    let (local_id, tenant, _hub, _paired) = local_identity();
    let owner_ura = crate::core::ura::device_ura(&tenant, &local_id);
    let mutated_by = require_local_device_authority(&env, &owner_ura, "ability.deploy")?;
    if !is_local_target(node_id, &local_id) {
        return Err(federation_not_wired(&format!(
            "deploying an ability to remote node {node_id:?}"
        )));
    }

    // ── manifest materialization ────────────────────────────────────
    let bundle = AbilityBundle::from_resource_ref(&args)?;
    let key = bundle.wire_key();
    let ability_ura = crate::core::ura::owner_ability_ura(&owner_ura, &key)
        .ok_or_else(|| anyhow::anyhow!("ability.deploy: cannot derive ability_ura"))?;
    let install_id = DeviceAbilityRecord::derive_install_id(
        &ability_ura,
        &manifest_digest(&bundle.manifest_bytes),
    );

    // The registrar (runtime binding + durable commit) must be wired.
    let Some(registrar) = device_registrar.get().cloned() else {
        anyhow::bail!(
            "ability.deploy: device registrar not wired yet (daemon still booting); \
             retry once the runtime is up"
        );
    };

    // Operator-facing store timestamp. The cryptographic execution
    // timeline remains the Axon receipt chain; this field is for deploy
    // listing, replay diagnostics, and deterministic sorting.
    let install = DeviceAbilityInstall::new(
        key.clone(),
        bundle.namespace.as_str(),
        ability_ura.clone(),
        bundle.manifest_path.clone(),
        bundle.manifest_bytes.clone(),
        bundle.manifest.clone(),
        clock.now_unix_ms(),
    )?;

    // ── runtime binding + route check + durable commit (transaction) ─
    let state = block_on_install(registrar, install)?;

    Ok(json!({
        "public_name": bundle.public_name,
        "namespace": bundle.namespace.as_str(),
        "ability_ura": ability_ura,
        "node_id": local_id,
        "mutated_by": mutated_by,
        "install_id": install_id,
        "bundle": bundle.display_path,
        // ACTIVE iff route resolver confirms the key is routable with
        // the expected call mode AND the durable commit succeeded.
        // Otherwise INSTALLED — never a false ACTIVE.
        "state": state.as_wire(),
    }))
}

/// Drive the registrar's async install from this sync handler. Spawns
/// onto the ambient runtime's workers (not `block_in_place`) so the
/// stream-source IO registers on the live driver — same rationale as
/// `mcp_executor::block_on_async`.
fn block_on_install(
    registrar: Arc<DeviceAbilityRegistrar>,
    install: DeviceAbilityInstall,
) -> anyhow::Result<
    crate::daemon::ability::builtins::device_control::ability_management::registrar::InstallState,
> {
    block_on_device_transaction(
        "ability.deploy",
        async move { registrar.install(install).await },
    )
}

// ── ability.uninstall ──────────────────────────────────────

fn uninstall_ability_handler(
    env: EnvelopeContext,
    args: Value,
    device_registrar: &SharedDeviceRegistrarCell,
) -> anyhow::Result<Value> {
    let ability_ura = args
        .get("ability_ura")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("ability.uninstall: `ability_ura` is required"))?;
    let public_name = ability_public_name(ability_ura)?;
    let node_id = args
        .get("node_id")
        .and_then(Value::as_str)
        .unwrap_or("local")
        .trim();
    let (local_id, tenant, _hub, _paired) = local_identity();
    let owner_ura = crate::core::ura::device_ura(&tenant, &local_id);
    let mutated_by = require_local_device_authority(&env, &owner_ura, "ability.uninstall")?;
    if !is_local_target(node_id, &local_id) {
        return Err(federation_not_wired(&format!(
            "uninstalling ability {ability_ura:?} from remote node {node_id:?}"
        )));
    }

    let Some(registrar) = device_registrar.get().cloned() else {
        anyhow::bail!(
            "ability.uninstall: device registrar not wired yet (daemon still booting); \
             retry once the runtime is up"
        );
    };
    let install_id = args
        .get("install_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let outcome = block_on_uninstall(
        registrar,
        DeviceAbilityUninstall {
            ability_ura: ability_ura.to_string(),
            install_id,
        },
    )?;

    Ok(json!({
        "public_name": public_name,
        "ability_ura": ability_ura,
        "node_id": local_id,
        "mutated_by": mutated_by,
        "install_ids": outcome.install_ids,
        "runtime_removed": outcome.runtime_removed,
        "control_plane_removed": outcome.control_plane_removed,
        "state": "REMOVED",
    }))
}

fn block_on_uninstall(
    registrar: Arc<DeviceAbilityRegistrar>,
    uninstall: DeviceAbilityUninstall,
) -> anyhow::Result<crate::daemon::ability::builtins::device_control::ability_management::registrar::DeviceAbilityUninstallOutcome>
{
    block_on_device_transaction("ability.uninstall", async move {
        registrar.uninstall(uninstall).await
    })
}

fn block_on_device_transaction<T, F>(surface: &'static str, fut: F) -> anyhow::Result<T>
where
    T: Send + 'static,
    F: std::future::Future<Output = anyhow::Result<T>> + Send + 'static,
{
    run_blocking(fut, NoRuntimeFallback::BuildCurrentThreadTokio)
        .map_err(|e| anyhow::anyhow!("{surface}: {e}"))
}

fn ability_public_name(ability_ura: &str) -> anyhow::Result<String> {
    let parsed = crate::core::ura::parse_ura(ability_ura)
        .map_err(|e| anyhow::anyhow!("ability.uninstall: invalid `ability_ura`: {e}"))?;
    if parsed.kind != crate::core::ura::URAKind::Ability {
        anyhow::bail!("ability.uninstall: `ability_ura` must be an Ability URA");
    }
    crate::core::ura::ability_name_from_parts(&parsed).ok_or_else(|| {
        anyhow::anyhow!("ability.uninstall: ability_ura `{ability_ura}` has no public ability name")
    })
}

// ── Discovery surfaces ───────────────────────────────────────────

pub fn list_nodes_description() -> &'static str {
    "List device nodes visible from this daemon. The handler resolves \
     the realm directory through federation.resolve and then directly \
     probes each discovered device-profile Agent with observe.health, \
     so callers can distinguish a local-only view, a directory-only view, \
     and a directly reachable peer."
}

pub fn list_nodes_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {}
    })
}

pub fn describe_node_description() -> &'static str {
    "Describe one node by id from the same live federation-backed view \
     used by node.list. Accepts `local`, this device's actual \
     node id, or any resolved peer node id."
}

pub fn describe_node_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["node_id"],
        "properties": {
            "node_id": { "type": "string" }
        }
    })
}

pub fn remove_node_description() -> &'static str {
    "Remove a node from the device set. Refuses to remove the local device \
     (use `easynet device reset` for that). Remote removal awaits the \
     federation Invoke replacement."
}

pub fn remove_node_input_schema() -> Value {
    describe_node_input_schema()
}

pub fn deploy_ability_description() -> &'static str {
    "Publish a host_stream device ability bundle ResourceRef to a node. Local \
     target validates the manifest, durably installs it, binds the runtime, \
     and registers the control-plane record. Remote targets defer to the \
     federation Invoke replacement. Shell and arbitrary host-command exec kinds \
     are rejected until a permission broker exists."
}

pub fn deploy_ability_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["resource_ref"],
        "properties": {
            "resource_ref": filesystem::resource_ref_schema(),
            "node_id": { "type": "string" }
        }
    })
}

pub fn uninstall_ability_description() -> &'static str {
    "Uninstall an ability from a node. Mirrors `ability.deploy`: \
     local target removes the durable row, runtime binding, and \
     control-plane record; remote targets are queued for the \
     federation Invoke replacement."
}

pub fn uninstall_ability_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["ability_ura"],
        "properties": {
            "ability_ura":  { "type": "string" },
            "node_id":      { "type": "string" },
            "install_id":   { "type": "string" }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_nodes_returns_at_least_self() {
        let resolver = detached_resolver();
        let resp = list_nodes_handler(json!({}), resolver.as_ref()).unwrap();
        let nodes = resp.get("nodes").and_then(Value::as_array).unwrap();
        assert!(
            nodes.iter().any(|n| n.get("is_self") == Some(&json!(true))),
            "node.list must include the local device entry: {resp}"
        );
        assert!(resp.get("federation_view").is_some());
    }

    #[test]
    fn registration_publishes_device_ops_manifests() {
        let mut reg = AxonAbilityCatalog::new();
        register(
            &mut reg,
            Arc::new(empty_device_cell()),
            Arc::new(populated_catalog_cell()),
            detached_resolver(),
        );

        for ability in [
            ABILITY_LIST_NODES,
            ABILITY_DESCRIBE_NODE,
            ABILITY_REMOVE_NODE,
            ABILITY_DEPLOY_ABILITY,
            ABILITY_UNINSTALL_ABILITY,
        ] {
            let record = reg
                .control_plane_record_for_mode(ability, crate::daemon::ability::CallMode::Rpc)
                .expect("control-plane lookup must be unambiguous")
                .unwrap_or_else(|| panic!("{ability} must publish a canonical descriptor"));
            assert_eq!(
                record
                    .descriptor()
                    .input_schema()
                    .get("type")
                    .and_then(Value::as_str),
                Some("object"),
                "{ability} must publish an object input schema"
            );
        }
    }

    #[test]
    fn describe_node_with_local_returns_self_envelope() {
        // HomeGuard isolates ~/.easynet so the handler runs the
        // unpaired-fallback arm (collect_device_view's self node).
        // The paired arm goes through federation_probe::resolve_device_record
        // which dials the local runtime bridge — absent in unit tests.
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let resolver = detached_resolver();
        let catalog = populated_catalog_cell();
        let resp = describe_node_handler(json!({"node_id": "local"}), resolver.as_ref(), &catalog)
            .unwrap();
        assert_eq!(resp.get("is_self"), Some(&json!(true)));
    }

    #[test]
    fn describe_node_with_remote_returns_not_found() {
        // Same HomeGuard isolation: unpaired fallback bails
        // "node X not found" without reaching the runtime bridge.
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let resolver = detached_resolver();
        let catalog = populated_catalog_cell();
        let err = describe_node_handler(
            json!({"node_id": "some-remote"}),
            resolver.as_ref(),
            &catalog,
        )
        .unwrap_err();
        assert!(format!("{err}").contains("not found"));
    }

    #[test]
    fn remove_node_refuses_to_remove_self() {
        let err = remove_node_handler(json!({"node_id": "local"})).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("device reset"),
            "must point at `easynet device reset`; got: {msg}"
        );
    }

    /// An unpopulated registrar cell — exercises the pre-binding
    /// validation path (resource_ref / manifest parse) without needing
    /// a live runtime. The install transaction itself is covered by the
    /// negative-test matrix with a wired runtime.
    fn empty_device_cell() -> SharedDeviceRegistrarCell {
        std::sync::OnceLock::new()
    }

    fn populated_catalog_cell() -> OnceLock<Arc<AxonAbilityCatalog>> {
        let cell = OnceLock::new();
        cell.set(Arc::new(AxonAbilityCatalog::new()))
            .expect("test catalog cell has one writer");
        cell
    }

    fn detached_resolver() -> SharedDiscoverFederationResolver {
        Arc::new(
            crate::daemon::ability::builtins::agents::discover::DetachedDiscoverFederationResolver,
        )
    }

    fn local_device_env() -> EnvelopeContext {
        EnvelopeContext::for_test(LOCAL_SYSTEM_AGENT_URA, "easynet:///r/test/device/local")
    }

    #[test]
    fn deploy_ability_rejects_missing_resource_ref() {
        let err = deploy_ability_handler(local_device_env(), json!({}), &empty_device_cell())
            .unwrap_err();
        assert!(format!("{err}").contains("resource_ref"));
    }

    #[test]
    fn deploy_ability_local_validates_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let resource_ref =
            filesystem::resource_ref_for_local_path(dir.path(), FilesystemResourceCapability::Read)
                .unwrap();
        let err = deploy_ability_handler(
            local_device_env(),
            json!({ "resource_ref": resource_ref, "node_id": "local" }),
            &empty_device_cell(),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("ability.json"));
    }

    #[test]
    fn deploy_ability_parses_canonical_manifest_then_needs_registrar() {
        // New contract: a well-formed manifest parses (verb-only name,
        // schema, exec), then the transaction needs a wired registrar.
        // With an empty cell the handler fails honestly at the binding
        // step — it does NOT report a false ACTIVE.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ability.json"),
            r#"{"name":"weather","namespace":"er","description":"w",
                "input_schema":{"type":"object"},
                "exec":{"kind":"shell","argv":["echo","hi"]}}"#,
        )
        .unwrap();
        let resource_ref =
            filesystem::resource_ref_for_local_path(dir.path(), FilesystemResourceCapability::Read)
                .unwrap();
        let err = deploy_ability_handler(
            local_device_env(),
            json!({ "resource_ref": resource_ref, "node_id": "local" }),
            &empty_device_cell(),
        )
        .unwrap_err();
        // Honest failure (registrar not wired), never a fake ACTIVE.
        assert!(
            format!("{err}").contains("registrar not wired"),
            "expected an honest not-wired failure, got: {err}"
        );
    }

    #[test]
    fn deploy_ability_rejects_missing_namespace_before_registrar() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ability.json"),
            r#"{"name":"weather","description":"w",
                "input_schema":{"type":"object"},
                "exec":{"kind":"shell","argv":["echo","hi"]}}"#,
        )
        .unwrap();
        let resource_ref =
            filesystem::resource_ref_for_local_path(dir.path(), FilesystemResourceCapability::Read)
                .unwrap();
        let err = deploy_ability_handler(
            local_device_env(),
            json!({ "resource_ref": resource_ref, "node_id": "local" }),
            &empty_device_cell(),
        )
        .unwrap_err();
        assert!(
            format!("{err}").contains("must declare non-empty `namespace`"),
            "{err}"
        );
    }

    #[test]
    fn deploy_ability_rejects_reserved_namespace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ability.json"),
            r#"{"name":"weather","namespace":"device","description":"w",
                "input_schema":{"type":"object"},
                "exec":{"kind":"shell","argv":["echo","hi"]}}"#,
        )
        .unwrap();
        let resource_ref =
            filesystem::resource_ref_for_local_path(dir.path(), FilesystemResourceCapability::Read)
                .unwrap();
        let err = deploy_ability_handler(
            local_device_env(),
            json!({ "resource_ref": resource_ref, "node_id": "local" }),
            &empty_device_cell(),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("reserved"), "{err}");
    }

    #[test]
    fn uninstall_ability_needs_wired_registrar() {
        let err = uninstall_ability_handler(
            local_device_env(),
            json!({
                "ability_ura": "easynet:///r/localhost/ability/alice.claude.weather",
                "node_id": "local",
            }),
            &empty_device_cell(),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("registrar not wired"));
    }

    #[test]
    fn deploy_ability_wired_transaction_completes_inside_current_thread_runtime() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ability.json"),
            r#"{"name":"weather","namespace":"er","description":"w",
                "admission_action":"stream","input_schema":{"type":"object"},
                "exec":{"kind":"host_stream","host_socket":"/tmp/er-host.sock","function":"er.weather"}}"#,
        )
        .unwrap();
        let resource_ref =
            filesystem::resource_ref_for_local_path(dir.path(), FilesystemResourceCapability::Read)
                .unwrap();
        let store_path = dir.path().join("device-abilities.json");
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let dir_path = dir.path().to_path_buf();

        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let result = runtime.block_on(async move {
                let registrar = DeviceAbilityRegistrar::new_pending_with_store(
                    DeviceAbilityStore::open_at(store_path),
                );
                let local_runtime =
                    crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
                        crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
                        None,
                    );
                let catalog = Arc::new(AxonAbilityCatalog::new_with_runtime(Arc::clone(
                    &local_runtime,
                )));
                registrar.set_runtime(local_runtime).unwrap();
                registrar
                    .set_control_plane_catalog(Arc::downgrade(&catalog))
                    .unwrap();
                let cell = SharedDeviceRegistrarCell::new();
                assert!(cell.set(registrar).is_ok());
                deploy_ability_handler(
                    local_device_env(),
                    json!({ "resource_ref": resource_ref, "node_id": "local" }),
                    &cell,
                )
            });
            let _ = tx.send(result);
            let _ = dir_path;
        });

        let resp = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("deploy transaction must not park the current-thread runtime")
            .unwrap();
        assert_eq!(resp.get("state").and_then(Value::as_str), Some("ACTIVE"));
    }
}
