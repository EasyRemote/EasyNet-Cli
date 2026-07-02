// EasyNet CLI — system ability manifest projection
// =================================================
//
// File: src/runtime/system_ability_catalog/system_manifest.rs
// Description: Converts daemon-owned ability registry keys into
//              `AbilityManifest` metadata used by the control plane.

use serde_json::Value;

use crate::core::ability_spec::AbilityManifest;

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
    let manifest_name = ability_key.rsplit('.').next().unwrap_or(ability_key);
    AbilityManifest::new(manifest_name, description, input_schema)
        .unwrap_or_else(|error| panic!("{ability_key} system manifest must be valid: {error}"))
}
