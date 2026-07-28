// EasyNet CLI — device node and ability management abilities
// ==========================================
//
// File: src/daemon/ability/builtins/device_control/ability_management/ops.rs
// Description: Device-hosted abilities the CLI's device/ability
//              subcommands invoke. Local device mutations are backed
//              by the canonical device ability registrar. Remote
//              device mutations are an explicit Unsupported capability
//              state until a provider-backed federation mutation route
//              exists in the runtime capability matrix.
//
// Abilities registered here
// -------------------------
//   node.describe     Describe one node by id.
//   node.remove       Remove a node from the realm device registry.
//   ability.deploy    Publish an ability bundle to a target Device URA.
//   ability.uninstall Uninstall a previously deployed ability.
//
// Routing model
// -------------
// Every mutation handler classifies a canonical target identity through
// `DeviceOperationTarget`. Local mutations require this device's Device URA.
// Other Device URAs are explicit remote targets and fail closed as Unsupported
// until the capability matrix marks remote mutation provider-backed.
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
use crate::support::async_bridge::{run_blocking, SyncBridgeRuntimePolicy};

/// Shared, late-wired cell holding the device-ability registrar.
/// Constructed pending at registry-build time; boot attaches the live
/// `LocalRuntime` via `set_runtime`. `ability.deploy`'s handler reads
/// it to run the install transaction. Mirrors
/// `agent_lifecycle_ability::SharedHotRegistrarCell`.
pub type SharedDeviceRegistrarCell = OnceLock<Arc<DeviceAbilityRegistrar>>;

pub const ABILITY_DESCRIBE_NODE: &str =
    crate::daemon::ability::names::device_control::NODE_DESCRIBE;
pub const ABILITY_REMOVE_NODE: &str = crate::daemon::ability::names::device_control::NODE_REMOVE;
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum DeviceOperationTarget {
    Local,
    RemoteUnsupported { target: String },
}

impl DeviceOperationTarget {
    fn classify_node_id(node_id: &str, local_node_id: &str) -> Self {
        let trimmed = node_id.trim();
        if trimmed.is_empty() || trimmed == "local" || trimmed == local_node_id {
            Self::Local
        } else {
            Self::RemoteUnsupported {
                target: trimmed.to_string(),
            }
        }
    }

    fn classify_target_ura(target_ura: &str, local_device_ura: &str) -> anyhow::Result<Self> {
        let target_ura = target_ura.trim();
        if target_ura.is_empty() {
            anyhow::bail!("device mutation target_ura must not be empty");
        }
        let parsed = crate::core::ura::parse_ura(target_ura).map_err(|error| {
            anyhow::anyhow!("device mutation target_ura must be canonical: {error}")
        })?;
        if parsed.kind != crate::core::ura::URAKind::Device {
            anyhow::bail!(
                "device mutation target_ura must identify a Device, got {:?}",
                parsed.kind
            );
        }
        if target_ura == local_device_ura {
            Ok(Self::Local)
        } else {
            Ok(Self::RemoteUnsupported {
                target: target_ura.to_string(),
            })
        }
    }

    fn is_local(&self) -> bool {
        matches!(self, Self::Local)
    }

    fn require_local_mutation(&self, surface: &str) -> anyhow::Result<()> {
        match self {
            Self::Local => Ok(()),
            Self::RemoteUnsupported { target } => Err(anyhow::anyhow!(
                "{surface}: remote device target {target:?} is unsupported by the canonical \
                 runtime capability matrix; capability_state=unsupported"
            )),
        }
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

#[derive(Debug, Clone)]
struct LocalDeviceIdentity {
    node_id: String,
    tenant_id: String,
}

/// Resolve the local node's identity from canonical runtime credentials.
/// Device-management operations require an explicit local owner; they must not
/// synthesize local rows when credentials are unavailable.
fn local_identity() -> anyhow::Result<LocalDeviceIdentity> {
    match federation_probe::local_identity() {
        federation_probe::LocalIdentity::Paired {
            node_id,
            tenant_id,
            hub_endpoint: _,
        } => Ok(LocalDeviceIdentity { node_id, tenant_id }),
        federation_probe::LocalIdentity::Unavailable { reason } => {
            anyhow::bail!("device operation local identity unavailable: {reason}")
        }
    }
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

fn require_device_registrar(
    device_registrar: &SharedDeviceRegistrarCell,
    surface: &str,
) -> anyhow::Result<Arc<DeviceAbilityRegistrar>> {
    device_registrar.get().cloned().ok_or_else(|| {
        anyhow::anyhow!(
            "{surface}: canonical device ability registrar is unavailable; \
             daemon runtime assembly has not completed"
        )
    })
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
    let local = local_identity()?;
    if DeviceOperationTarget::classify_node_id(node_id, &local.node_id).is_local() {
        let catalog = local_catalog.get().ok_or_else(|| {
            anyhow::anyhow!("node.describe: live ability catalog is not attached")
        })?;
        if let Some(record) = federation_probe::local_device_record(catalog.as_ref())? {
            return Ok(node_json_with_abilities(
                &record.node,
                record.ability_summaries,
            ));
        }
        if let Some(record) = federation_probe::resolve_device_record(resolver, &local.node_id)? {
            return Ok(node_json_with_abilities(
                &record.node,
                record.ability_summaries,
            ));
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
    let local = local_identity()?;
    let target = DeviceOperationTarget::classify_node_id(node_id, &local.node_id);
    match &target {
        DeviceOperationTarget::Local => {
            anyhow::bail!(
                "node.remove refuses to remove this device (would delete its own \
                 pairing). Use `easynet device reset` for that — it is the local \
                 side of the same operation."
            );
        }
        DeviceOperationTarget::RemoteUnsupported { .. } => {
            target.require_local_mutation("node.remove")?;
            unreachable!("remote target classification must return Unsupported before mutation")
        }
    }
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
    /// Canonical manifest bytes (for the durable manifest_hash). The deploy
    /// envelope's `namespace` field is intentionally removed before hashing,
    /// storing, restoring, or binding descriptors.
    manifest_bytes: Vec<u8>,
    /// The deserialized canonical manifest. The deploy bundle may carry
    /// `namespace` beside it; arbitrary provider metadata is not part of the
    /// runtime manifest and fails closed before registrar mutation.
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

        let raw_manifest_bytes = std::fs::read(&manifest_file)?;
        let (manifest, namespace, manifest_bytes) =
            parse_device_ability_bundle_manifest(&raw_manifest_bytes, &display_path)?;

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

fn parse_device_ability_bundle_manifest(
    bytes: &[u8],
    display_path: &str,
) -> anyhow::Result<(
    crate::daemon::ability::manifest::AbilityManifest,
    AbilityNamespace,
    Vec<u8>,
)> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|e| anyhow::anyhow!("invalid ability.json at {display_path}/ability.json: {e}"))?;
    let Value::Object(mut object) = value else {
        anyhow::bail!("invalid ability.json at {display_path}/ability.json: expected JSON object");
    };

    let namespace_value = object.remove("namespace");
    let namespace = match namespace_value {
        Some(Value::String(namespace)) => AbilityNamespace::parse(Some(namespace.as_str()))?,
        Some(_) => anyhow::bail!("ability.deploy: ability.json `namespace` must be a string"),
        None => AbilityNamespace::parse(None)?,
    };

    let canonical_value = Value::Object(object);
    let canonical_manifest_bytes = serde_json::to_vec(&canonical_value).map_err(|e| {
        anyhow::anyhow!("invalid ability.json at {display_path}/ability.json: serialize canonical manifest: {e}")
    })?;
    let manifest = crate::daemon::ability::manifest::AbilityManifest::from_json_slice(
        &canonical_manifest_bytes,
    )
    .map_err(|e| anyhow::anyhow!("invalid ability.json at {display_path}/ability.json: {e}"))?;

    Ok((manifest, namespace, canonical_manifest_bytes))
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
    let target_ura = args
        .get("target_ura")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("ability.deploy: `target_ura` is required"))?;
    let local = local_identity()?;
    let owner_ura = crate::core::ura::device_ura(&local.tenant_id, &local.node_id);
    let mutated_by = require_local_device_authority(&env, &owner_ura, "ability.deploy")?;
    DeviceOperationTarget::classify_target_ura(target_ura, &owner_ura)?
        .require_local_mutation("ability.deploy")?;

    // ── manifest materialization ────────────────────────────────────
    let bundle = AbilityBundle::from_resource_ref(&args)?;
    let key = bundle.wire_key();
    let ability_ura = crate::core::ura::owner_ability_ura(&owner_ura, &key)
        .ok_or_else(|| anyhow::anyhow!("ability.deploy: cannot derive ability_ura"))?;
    let install_id = DeviceAbilityRecord::derive_install_id(
        &ability_ura,
        &manifest_digest(&bundle.manifest_bytes),
    );

    // The registrar (runtime binding + durable commit) is a canonical daemon
    // runtime assembly precondition.
    let registrar = require_device_registrar(device_registrar, "ability.deploy")?;

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
        "node_id": local.node_id,
        "target_ura": owner_ura,
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
    let target_ura = args
        .get("target_ura")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("ability.uninstall: `target_ura` is required"))?;
    let local = local_identity()?;
    let owner_ura = crate::core::ura::device_ura(&local.tenant_id, &local.node_id);
    let mutated_by = require_local_device_authority(&env, &owner_ura, "ability.uninstall")?;
    DeviceOperationTarget::classify_target_ura(target_ura, &owner_ura)?
        .require_local_mutation("ability.uninstall")?;

    let registrar = require_device_registrar(device_registrar, "ability.uninstall")?;
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
        "node_id": local.node_id,
        "target_ura": owner_ura,
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
    run_blocking(fut, SyncBridgeRuntimePolicy::BuildCurrentThreadTokio)
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

pub fn describe_node_description() -> &'static str {
    "Describe one node by id from the same live federation-backed view \
     used by federation.discover. Accepts `local`, this device's actual \
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
     (use `easynet device reset` for that). Remote removal is currently \
     unsupported by the canonical runtime capability matrix."
}

pub fn remove_node_input_schema() -> Value {
    describe_node_input_schema()
}

pub fn deploy_ability_description() -> &'static str {
    "Publish a host_stream device ability bundle ResourceRef to a canonical Device URA. Local \
     target validates the manifest, durably installs it, binds the runtime, \
     and registers the control-plane record. Remote targets are currently \
     unsupported by the canonical runtime capability matrix. Shell and arbitrary \
     host-command exec kinds are rejected until a permission broker exists."
}

pub fn deploy_ability_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["resource_ref", "target_ura"],
        "properties": {
            "resource_ref": filesystem::resource_ref_schema(),
            "target_ura": { "type": "string" }
        }
    })
}

pub fn uninstall_ability_description() -> &'static str {
    "Uninstall an ability from a canonical Device URA. Mirrors `ability.deploy`: \
     local target removes the durable row, runtime binding, and \
     control-plane record; remote targets are currently unsupported by the \
     canonical runtime capability matrix."
}

pub fn uninstall_ability_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["ability_ura", "target_ura"],
        "properties": {
            "ability_ura":  { "type": "string" },
            "target_ura":   { "type": "string" },
            "install_id":   { "type": "string" }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_DEVICE_URA: &str = "easynet:///r/test/device/local";

    fn metadata_test_catalog() -> AxonAbilityCatalog {
        AxonAbilityCatalog::new_test_metadata_for_device_authority(TEST_DEVICE_URA)
    }

    fn runtime_test_catalog(
        runtime: Arc<axon_sdk::invocation::LocalRuntime>,
    ) -> Arc<AxonAbilityCatalog> {
        Arc::new(AxonAbilityCatalog::new_test_runtime_for_device_authority(
            runtime,
            TEST_DEVICE_URA,
        ))
    }

    #[test]
    fn registration_publishes_device_ops_manifests() {
        let _home = provision_local_device_credentials();
        let mut reg = metadata_test_catalog();
        register(
            &mut reg,
            Arc::new(empty_device_cell()),
            Arc::new(populated_catalog_cell()),
            detached_resolver(),
        );

        for ability in [
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
        let _home = provision_local_device_credentials();
        let resolver = detached_resolver();
        let catalog = populated_catalog_cell();
        let resp = describe_node_handler(json!({"node_id": "local"}), resolver.as_ref(), &catalog)
            .unwrap();
        assert_eq!(resp.get("is_self"), Some(&json!(true)));
    }

    #[test]
    fn describe_node_with_remote_reports_federation_unavailable() {
        let _home = provision_local_device_credentials();
        let resolver = detached_resolver();
        let catalog = populated_catalog_cell();
        let err = describe_node_handler(
            json!({"node_id": "some-remote"}),
            resolver.as_ref(),
            &catalog,
        )
        .unwrap_err();
        assert!(
            format!("{err}").contains("federation.resolve"),
            "remote describe must preserve resolver failure instead of returning fallback not-found: {err}"
        );
    }

    #[test]
    fn remove_node_refuses_to_remove_self() {
        let _home = provision_local_device_credentials();
        let err = remove_node_handler(json!({"node_id": "local"})).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("device reset"),
            "must point at `easynet device reset`; got: {msg}"
        );
    }

    #[test]
    fn remove_node_remote_target_is_unsupported_capability_state() {
        let _home = provision_local_device_credentials();
        let err = remove_node_handler(json!({"node_id": "remote-a"})).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("capability_state=unsupported"),
            "remote mutation must fail as an explicit capability state: {msg}"
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
        cell.set(Arc::new(metadata_test_catalog()))
            .expect("test catalog cell has one writer");
        cell
    }

    fn detached_resolver() -> SharedDiscoverFederationResolver {
        Arc::new(
            crate::daemon::ability::builtins::agents::discover::DetachedDiscoverFederationResolver,
        )
    }

    fn provision_local_device_credentials() -> crate::cli::commands::test_support::HomeGuard {
        let guard = crate::cli::commands::test_support::HomeGuard::new();
        crate::daemon::persistence::config::save_credentials(
            &crate::daemon::persistence::config::Credentials {
                node_id: "local".to_string(),
                credential_token: "token".to_string(),
                hub_endpoint: "axon://hub.example:50051".to_string(),
                realm: "test".to_string(),
                username: Some("alice".to_string()),
                user_id: Some("user-alice".to_string()),
                ..Default::default()
            },
        )
        .expect("write local device credentials");
        guard
    }

    fn local_device_env() -> EnvelopeContext {
        EnvelopeContext::for_test(LOCAL_SYSTEM_AGENT_URA, "easynet:///r/test/device/local")
    }

    #[test]
    fn deploy_ability_rejects_missing_resource_ref() {
        let _home = provision_local_device_credentials();
        let err = deploy_ability_handler(
            local_device_env(),
            json!({ "target_ura": TEST_DEVICE_URA }),
            &empty_device_cell(),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("resource_ref"));
    }

    #[test]
    fn deploy_ability_requires_canonical_target_ura() {
        let _home = provision_local_device_credentials();
        for args in [
            json!({ "resource_ref": {} }),
            json!({ "resource_ref": {}, "target_ura": "local" }),
            json!({ "resource_ref": {}, "target_ura": "remote-a" }),
            json!({ "resource_ref": {}, "target_ura": crate::core::ura::hub_ura("test") }),
        ] {
            let err = deploy_ability_handler(local_device_env(), args, &empty_device_cell())
                .expect_err("deploy target must be an explicit canonical Device URA");
            let message = format!("{err}");
            assert!(
                message.contains("target_ura"),
                "target validation must name target_ura: {message}"
            );
        }
    }

    #[test]
    fn deploy_ability_local_validates_manifest() {
        let _home = provision_local_device_credentials();
        let dir = tempfile::tempdir().unwrap();
        let resource_ref =
            filesystem::resource_ref_for_local_path(dir.path(), FilesystemResourceCapability::Read)
                .unwrap();
        let err = deploy_ability_handler(
            local_device_env(),
            json!({ "resource_ref": resource_ref, "target_ura": TEST_DEVICE_URA }),
            &empty_device_cell(),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("ability.json"));
    }

    #[test]
    fn deploy_ability_parses_canonical_manifest_then_requires_registrar() {
        let _home = provision_local_device_credentials();
        // New contract: a well-formed manifest parses (verb-only name,
        // schema, exec), then the transaction requires the canonical device
        // ability registrar. With an empty cell the handler fails honestly at
        // the runtime assembly step — it does NOT report a false ACTIVE.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ability.json"),
            r#"{"schema_version":"1","name":"weather","namespace":"er","description":"w",
                "input_schema":{"type":"object"},
                "exec":{"kind":"shell","argv":["echo","hi"]}}"#,
        )
        .unwrap();
        let resource_ref =
            filesystem::resource_ref_for_local_path(dir.path(), FilesystemResourceCapability::Read)
                .unwrap();
        let err = deploy_ability_handler(
            local_device_env(),
            json!({ "resource_ref": resource_ref, "target_ura": TEST_DEVICE_URA }),
            &empty_device_cell(),
        )
        .unwrap_err();
        assert!(
            format!("{err}").contains("canonical device ability registrar is unavailable"),
            "expected canonical registrar precondition failure, got: {err}"
        );
    }

    #[test]
    fn deploy_ability_remote_target_is_unsupported_before_bundle_materialization() {
        let _home = provision_local_device_credentials();
        let err = deploy_ability_handler(
            local_device_env(),
            json!({ "target_ura": "easynet:///r/test/device/remote-a" }),
            &empty_device_cell(),
        )
        .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("capability_state=unsupported") && !msg.contains("resource_ref"),
            "remote target must fail before local bundle materialization: {msg}"
        );
    }

    #[test]
    fn deploy_ability_rejects_missing_namespace_before_registrar() {
        let _home = provision_local_device_credentials();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ability.json"),
            r#"{"schema_version":"1","name":"weather","description":"w",
                "input_schema":{"type":"object"},
                "exec":{"kind":"shell","argv":["echo","hi"]}}"#,
        )
        .unwrap();
        let resource_ref =
            filesystem::resource_ref_for_local_path(dir.path(), FilesystemResourceCapability::Read)
                .unwrap();
        let err = deploy_ability_handler(
            local_device_env(),
            json!({ "resource_ref": resource_ref, "target_ura": TEST_DEVICE_URA }),
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
        let _home = provision_local_device_credentials();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ability.json"),
            r#"{"schema_version":"1","name":"weather","namespace":"device","description":"w",
                "input_schema":{"type":"object"},
                "exec":{"kind":"shell","argv":["echo","hi"]}}"#,
        )
        .unwrap();
        let resource_ref =
            filesystem::resource_ref_for_local_path(dir.path(), FilesystemResourceCapability::Read)
                .unwrap();
        let err = deploy_ability_handler(
            local_device_env(),
            json!({ "resource_ref": resource_ref, "target_ura": TEST_DEVICE_URA }),
            &empty_device_cell(),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("reserved"), "{err}");
    }

    #[test]
    fn deploy_ability_bundle_parser_strips_namespace_from_canonical_manifest_bytes() {
        let raw = br#"{"schema_version":"1","name":"weather","namespace":"er","description":"w",
            "input_schema":{"type":"object"},
            "exec":{"kind":"shell","argv":["echo","hi"]}}"#;

        let (manifest, namespace, canonical_bytes) =
            parse_device_ability_bundle_manifest(raw, "/tmp/bundle").unwrap();
        assert_eq!(manifest.name(), "weather");
        assert_eq!(namespace.as_str(), "er");
        let canonical: Value = serde_json::from_slice(&canonical_bytes).unwrap();
        assert!(
            canonical.get("namespace").is_none(),
            "canonical manifest bytes must not retain deploy-envelope namespace: {canonical}"
        );
        crate::daemon::ability::manifest::AbilityManifest::from_json_slice(&canonical_bytes)
            .expect("canonical bytes must parse as strict AbilityManifest");
    }

    #[test]
    fn deploy_ability_rejects_unknown_provider_metadata_before_registrar() {
        let _home = provision_local_device_credentials();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ability.json"),
            r#"{"schema_version":"1","name":"weather","namespace":"er","description":"w",
                "input_schema":{"type":"object"},
                "tool_name":"legacy-provider-field",
                "exec":{"kind":"shell","argv":["echo","hi"]}}"#,
        )
        .unwrap();
        let resource_ref =
            filesystem::resource_ref_for_local_path(dir.path(), FilesystemResourceCapability::Read)
                .unwrap();
        let err = deploy_ability_handler(
            local_device_env(),
            json!({ "resource_ref": resource_ref, "target_ura": TEST_DEVICE_URA }),
            &empty_device_cell(),
        )
        .unwrap_err();
        assert!(
            format!("{err}").contains("unknown field `tool_name`"),
            "provider metadata must fail at bundle parse before registrar mutation: {err}"
        );
    }

    #[test]
    fn uninstall_ability_requires_canonical_registrar() {
        let _home = provision_local_device_credentials();
        let err = uninstall_ability_handler(
            local_device_env(),
            json!({
                "ability_ura": "easynet:///r/localhost/ability/alice.claude.weather",
                "target_ura": TEST_DEVICE_URA,
            }),
            &empty_device_cell(),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("canonical device ability registrar is unavailable"));
    }

    #[test]
    fn uninstall_ability_remote_target_is_unsupported_capability_state() {
        let _home = provision_local_device_credentials();
        let err = uninstall_ability_handler(
            local_device_env(),
            json!({
                "ability_ura": "easynet:///r/localhost/ability/alice.claude.weather",
                "target_ura": "easynet:///r/test/device/remote-a",
            }),
            &empty_device_cell(),
        )
        .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("capability_state=unsupported"),
            "remote uninstall must fail as an explicit capability state: {msg}"
        );
    }

    #[test]
    fn deploy_ability_wired_transaction_completes_inside_current_thread_runtime() {
        let _home = provision_local_device_credentials();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ability.json"),
            r#"{"schema_version":"1","name":"weather","namespace":"er","description":"w",
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
                let catalog = runtime_test_catalog(Arc::clone(&local_runtime));
                registrar.set_runtime(local_runtime).unwrap();
                registrar
                    .set_control_plane_catalog(Arc::downgrade(&catalog))
                    .unwrap();
                let cell = SharedDeviceRegistrarCell::new();
                assert!(cell.set(registrar).is_ok());
                deploy_ability_handler(
                    local_device_env(),
                    json!({ "resource_ref": resource_ref, "target_ura": TEST_DEVICE_URA }),
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
