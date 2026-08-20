// EasyNet CLI - canonical ability catalog contract
// =================================================
//
// File: src/daemon/ability/catalog_row.rs
// Description: Owns the typed query, row, identity, and conflict gate shared by
//              daemon catalog projection, federation read models, CLI, and FFI.
//
// Protocol Responsibility:
// - Preserve owner, Ability URA, descriptor version, call mode, hash, and action
//   as one indivisible catalog identity.
// - Reject malformed or conflicting rows before any facade selects a descriptor.
//
// Implementation Approach:
// - Deserialize through the canonical AbilityDescriptor wire aggregate, whose
//   parser verifies every derived hash and descriptor-ref binding.
// - Keep the original JSON row after validation so additive public fields remain
//   available to callers without becoming a second descriptor authority.
//
// Usage Contract:
// - Every consumer of meta.list_abilities rows must call AbilityCatalogRow::parse.
// - Every catalog merge must use insert_catalog_descriptor.
//
// Architectural Position:
// - Daemon-owned catalog schema gate. Axon continues to own descriptor-ref
//   grammar and canonical Invocation semantics.

use std::collections::BTreeMap;

use serde_json::Value;

use super::{AbilityDescriptor, CallMode};

/// Public catalog-row compatibility alias for [`AbilityDescriptor::version`].
///
/// `version` is the canonical descriptor identity field. `descriptor_version`
/// remains only as an additive wire-adapter field for older catalog consumers and
/// descriptor-ref clients. It is never a second catalog identity: if present on
/// input it must equal `version`, and committed keys always use
/// [`CatalogDescriptorKey::descriptor_version`] derived from the descriptor.
const WIRE_DESCRIPTOR_VERSION_ALIAS: &str = "descriptor_version";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CatalogScope {
    Realm,
}

impl CatalogScope {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Realm => "realm",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AbilityCatalogQuery {
    scope: Option<CatalogScope>,
    owner_ura: Option<String>,
    ability_ura: Option<String>,
    descriptor_version: Option<String>,
}

impl AbilityCatalogQuery {
    pub(crate) fn new(owner_ura: Option<String>, ability_ura: Option<String>) -> Self {
        Self {
            scope: None,
            owner_ura,
            ability_ura,
            descriptor_version: None,
        }
    }

    pub(crate) fn exact(
        owner_ura: &str,
        ability_ura: &str,
        descriptor_version: Option<&str>,
    ) -> Self {
        Self {
            scope: Some(CatalogScope::Realm),
            owner_ura: Some(owner_ura.to_string()),
            ability_ura: Some(ability_ura.to_string()),
            descriptor_version: descriptor_version.map(str::to_string),
        }
    }

    pub(crate) fn all_realm() -> Self {
        Self {
            scope: Some(CatalogScope::Realm),
            ..Self::default()
        }
    }

    pub(crate) fn owner_ura(&self) -> Option<&str> {
        self.owner_ura.as_deref()
    }

    pub(crate) fn ability_ura(&self) -> Option<&str> {
        self.ability_ura.as_deref()
    }

    pub(crate) fn descriptor_version(&self) -> Option<&str> {
        self.descriptor_version.as_deref()
    }

    pub(crate) fn to_request_json(&self) -> Value {
        let mut request = serde_json::Map::new();
        if let Some(scope) = self.scope {
            request.insert(
                "scope".to_string(),
                Value::String(scope.as_str().to_string()),
            );
        }
        if let Some(owner_ura) = self.owner_ura.as_ref() {
            request.insert("owner_ura".to_string(), Value::String(owner_ura.clone()));
        }
        if let Some(ability_ura) = self.ability_ura.as_ref() {
            request.insert(
                "ability_ura".to_string(),
                Value::String(ability_ura.clone()),
            );
        }
        if let Some(version) = self.descriptor_version.as_ref() {
            request.insert(
                "descriptor_version".to_string(),
                Value::String(version.clone()),
            );
        }
        Value::Object(request)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CatalogDescriptorKey {
    owner_ura: String,
    ability_ura: String,
    descriptor_version: String,
    call_mode: CallMode,
}

impl CatalogDescriptorKey {
    pub(crate) fn from_descriptor(descriptor: &AbilityDescriptor) -> Result<Self, String> {
        let ability_ura = descriptor.canonical_ability_ura().ok_or_else(|| {
            format!(
                "descriptor owner {:?} and name {:?} do not derive a canonical Ability URA",
                descriptor.owner_ura,
                descriptor.public_name()
            )
        })?;
        Ok(Self {
            owner_ura: descriptor.owner_ura.clone(),
            ability_ura,
            descriptor_version: descriptor.version.clone(),
            call_mode: descriptor.call_mode(),
        })
    }

    pub(crate) fn owner_ura(&self) -> &str {
        &self.owner_ura
    }

    pub(crate) fn ability_ura(&self) -> &str {
        &self.ability_ura
    }

    pub(crate) fn descriptor_version(&self) -> &str {
        &self.descriptor_version
    }

    pub(crate) fn call_mode(&self) -> CallMode {
        self.call_mode
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AbilityCatalogRow {
    descriptor: AbilityDescriptor,
    value: Value,
}

impl AbilityCatalogRow {
    pub(crate) fn parse(value: &Value, index: usize, source: &str) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or_else(|| format!("{source} catalog row #{index} is not a JSON object"))?;
        for retired in ["ability_name", "tool_name", "public_name"] {
            if object.contains_key(retired) {
                return Err(format!(
                    "{source} catalog row #{index} contains retired alias field {retired:?}"
                ));
            }
        }
        for field in [
            "ability_ura",
            "owner_ura",
            "name",
            "version",
            "descriptor_hash",
            "call_mode",
            "admission_action",
            "descriptor_ref",
        ] {
            required_string(object, field)
                .map_err(|error| format!("{source} catalog row #{index} {error}"))?;
        }
        let descriptor: AbilityDescriptor =
            serde_json::from_value(value.clone()).map_err(|error| {
                format!(
                    "{source} catalog row #{index} is not a canonical AbilityDescriptor: {error}"
                )
            })?;
        if let Some(alias) = object
            .get(WIRE_DESCRIPTOR_VERSION_ALIAS)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if alias != descriptor.version {
                return Err(format!(
                    "{source} catalog row #{index} {WIRE_DESCRIPTOR_VERSION_ALIAS} {alias:?} does not match version {:?}",
                    descriptor.version
                ));
            }
        }
        CatalogDescriptorKey::from_descriptor(&descriptor).map_err(|error| {
            format!("{source} catalog row #{index} has invalid identity: {error}")
        })?;
        Ok(Self {
            descriptor,
            value: value.clone(),
        })
    }

    pub(crate) fn from_descriptor(descriptor: AbilityDescriptor) -> Result<Self, String> {
        let mut value = serde_json::to_value(&descriptor)
            .map_err(|error| format!("serialize canonical AbilityDescriptor: {error}"))?;
        if let Value::Object(object) = &mut value {
            object.insert(
                WIRE_DESCRIPTOR_VERSION_ALIAS.to_string(),
                Value::String(descriptor.version.clone()),
            );
        }
        Self::parse(&value, 0, "descriptor projection")
    }

    pub(crate) fn descriptor(&self) -> &AbilityDescriptor {
        &self.descriptor
    }

    pub(crate) fn key(&self) -> CatalogDescriptorKey {
        CatalogDescriptorKey::from_descriptor(&self.descriptor)
            .expect("validated AbilityCatalogRow must have a canonical key")
    }

    pub(crate) fn into_value(self) -> Value {
        self.value
    }
}

pub(crate) fn insert_catalog_descriptor(
    catalog: &mut BTreeMap<CatalogDescriptorKey, AbilityDescriptor>,
    descriptor: AbilityDescriptor,
    source: &str,
) -> Result<(), String> {
    let key = CatalogDescriptorKey::from_descriptor(&descriptor)?;
    match catalog.entry(key) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(descriptor);
            Ok(())
        }
        std::collections::btree_map::Entry::Occupied(entry) if entry.get() == &descriptor => Ok(()),
        std::collections::btree_map::Entry::Occupied(entry) => Err(format!(
            "{source} contains conflicting descriptors for owner {:?}, ability {:?}, version {:?}, mode {:?}: hashes {:?} and {:?}",
            entry.key().owner_ura(),
            entry.key().ability_ura(),
            entry.key().descriptor_version(),
            entry.key().call_mode().as_str(),
            entry.get().descriptor_hash_prefixed(),
            descriptor.descriptor_hash_prefixed(),
        )),
    }
}

fn required_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Result<&'a str, String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("missing required field {field:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::ability::descriptors::{AdmissionAction, Visibility};

    fn descriptor(mode: CallMode, version: &str) -> AbilityDescriptor {
        AbilityDescriptor::new(
            "chat",
            crate::core::ura::agent_ura("default", "user-a", "assistant"),
            Visibility::Public,
            AdmissionAction::Invoke,
        )
        .expect("descriptor")
        .with_call_mode(mode)
        .with_version(version)
        .expect("version")
    }

    #[test]
    fn row_round_trip_preserves_full_descriptor_identity() {
        let row = AbilityCatalogRow::from_descriptor(descriptor(CallMode::Stream, "2.0.0"))
            .expect("canonical row");
        let key = row.key();
        assert_eq!(key.descriptor_version(), "2.0.0");
        assert_eq!(key.call_mode(), CallMode::Stream);
        assert_eq!(row.into_value()["descriptor_version"], "2.0.0");
    }

    #[test]
    fn descriptor_version_alias_is_wire_adapter_only() {
        let mut value =
            serde_json::to_value(descriptor(CallMode::Rpc, "2.0.0")).expect("descriptor JSON");
        value.as_object_mut().expect("descriptor object").insert(
            WIRE_DESCRIPTOR_VERSION_ALIAS.to_string(),
            Value::String("9.9.9".to_string()),
        );

        let error = AbilityCatalogRow::parse(&value, 7, "test catalog")
            .expect_err("descriptor_version alias must not define a second identity");
        assert!(
            error.contains("descriptor_version")
                && error.contains("does not match version")
                && error.contains("2.0.0"),
            "{error}"
        );
    }

    #[test]
    fn conflict_gate_preserves_distinct_modes_and_rejects_same_key_drift() {
        let mut catalog = BTreeMap::new();
        let rpc = descriptor(CallMode::Rpc, "1.0.0");
        let stream = descriptor(CallMode::Stream, "1.0.0");
        insert_catalog_descriptor(&mut catalog, rpc.clone(), "test").unwrap();
        insert_catalog_descriptor(&mut catalog, stream, "test").unwrap();
        assert_eq!(catalog.len(), 2);

        let changed = rpc.with_description("changed without a version bump");
        let error = insert_catalog_descriptor(&mut catalog, changed, "test")
            .expect_err("same identity with a different hash must fail");
        assert!(error.contains("conflicting descriptors"));
    }
}
