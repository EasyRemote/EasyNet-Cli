// EasyNet CLI — system ability manifest projection
// =================================================
//
// File: src/daemon/ability/catalog/system_manifest.rs
// Description: Converts daemon-owned ability registry keys into
//              `AbilityManifest` metadata used by the control plane.

use serde_json::Value;

use crate::daemon::ability::manifest::{AbilityManifest, AccessPolicy, ManifestAccessScope};

pub(crate) fn canonical_registration_contract(
    ability_key: &str,
) -> anyhow::Result<super::SystemAbilityContract> {
    let path = super::system_ability_descriptor_path(ability_key);
    let body = std::fs::read_to_string(&path).map_err(|error| {
        anyhow::anyhow!(
            "read canonical descriptor contract for {ability_key:?} from {}: {error}",
            path.display()
        )
    })?;
    let contract = super::ability_toml::parse_ability_contract_toml(&body)?;
    if contract.name != ability_key {
        anyhow::bail!(
            "canonical descriptor {} names {:?}, expected {ability_key:?}",
            path.display(),
            contract.name
        );
    }
    Ok(contract)
}

/// Import the daemon's pure system metadata for a schema-less static
/// registration.
///
/// This is a registration-boundary adapter, not a discovery overlay. The
/// resulting manifest is immediately normalized into the control-plane
/// `AbilityDescriptor` and is never retained as a parallel read model.
pub(crate) fn registration_manifest(ability_key: &str) -> anyhow::Result<AbilityManifest> {
    let contract = canonical_registration_contract(ability_key)?;
    let manifest_name = ability_key.rsplit('.').next().unwrap_or(ability_key);
    let mut manifest = AbilityManifest::new(
        manifest_name,
        contract.description.clone(),
        contract.input_schema.clone(),
    )?;
    if !contract.output_receipt_schema.is_null() {
        manifest = manifest.with_output_schema(contract.output_receipt_schema.clone())?;
    }
    if ability_key.starts_with("observe.") {
        manifest = manifest.with_access(AccessPolicy {
            visibility: ManifestAccessScope::Public,
            ..Default::default()
        })?;
    }
    manifest
        .with_descriptor_version(contract.descriptor_version)?
        .with_admission_action(contract.admission_action.as_str())
}

/// Build the manifest metadata attached to a daemon-owned ability registration.
///
/// Invariant 1: `ability_key` is the catalog/control-plane key and may contain
/// namespace dots such as `invocation.history.list`.
/// Invariant 2: the manifest name is only the owner-local verb segment because
/// `AbilityManifest` forbids dots by design; the control-plane registration
/// still receives the full `ability_key` and therefore owns the public name.
/// Invariant 3: invalid static metadata fails at daemon boot instead of being
/// downgraded to a name-only descriptor.
pub(crate) fn registry_manifest(
    ability_key: &'static str,
    description: &'static str,
    input_schema: Value,
) -> AbilityManifest {
    let contract = canonical_registration_contract(ability_key).unwrap_or_else(|error| {
        panic!("{ability_key} canonical descriptor contract must be valid: {error}")
    });
    let manifest_name = ability_key.rsplit('.').next().unwrap_or(ability_key);
    let mut manifest = AbilityManifest::new(manifest_name, description, input_schema)
        .unwrap_or_else(|error| panic!("{ability_key} system manifest must be valid: {error}"));
    if !contract.output_receipt_schema.is_null() {
        manifest = manifest
            .with_output_schema(contract.output_receipt_schema.clone())
            .unwrap_or_else(|error| {
                panic!("{ability_key} output receipt schema must be valid: {error}")
            });
    }
    manifest
        .with_descriptor_version(contract.descriptor_version)
        .and_then(|manifest| manifest.with_admission_action(contract.admission_action.as_str()))
        .unwrap_or_else(|error| {
            panic!("{ability_key} governed manifest fields are invalid: {error}")
        })
}
