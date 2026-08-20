// EasyNet CLI - AbilityDescriptor registry facts
// ==============================================
//
// File: src/daemon/ability/descriptors/mod.rs
// Description: Versioned governed interface facts for daemon-published
//              abilities. These are local control-plane records, not handler
//              implementations.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::AbilityControlPlaneError;

mod surface;

pub(crate) use surface::{ability_hints_from_wire_json, ability_hints_wire_json};
pub use surface::{
    AbilityDescriptor, AbilityHints, AbilityIdentity, AbilitySchemaSummary, AdmissionAction,
    DescriptorError, ReceiptSemantics, ScopeRule, StateTransition, StateTransitionError,
    TransitionClass, Visibility,
};

pub const DEFAULT_ABILITY_DESCRIPTOR_VERSION: &str =
    crate::daemon::ability::manifest::DEFAULT_DESCRIPTOR_VERSION;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AbilityDescriptorVersion(String);

impl AbilityDescriptorVersion {
    pub fn new(version: impl Into<String>) -> Result<Self, AbilityControlPlaneError> {
        let version = version.into();
        if version.trim().is_empty() {
            return Err(AbilityControlPlaneError::EmptyDescriptorVersion);
        }
        if !is_valid_descriptor_version(&version) {
            return Err(AbilityControlPlaneError::InvalidDescriptorVersion { version });
        }
        Ok(Self(version))
    }

    pub(crate) fn from_validated(version: impl Into<String>) -> Self {
        Self(version.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for AbilityDescriptorVersion {
    fn default() -> Self {
        Self(DEFAULT_ABILITY_DESCRIPTOR_VERSION.to_string())
    }
}

impl std::fmt::Display for AbilityDescriptorVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AbilityDescriptorKey {
    ability: String,
    descriptor_version: AbilityDescriptorVersion,
    call_mode: CallMode,
}

impl AbilityDescriptorKey {
    pub fn new(
        ability: impl Into<String>,
        descriptor_version: impl Into<String>,
        call_mode: CallMode,
    ) -> Result<Self, AbilityControlPlaneError> {
        let ability = ability.into();
        if ability.trim().is_empty() {
            return Err(AbilityControlPlaneError::EmptyDescriptorName);
        }
        if !is_valid_ability_name(&ability) {
            return Err(AbilityControlPlaneError::InvalidDescriptorName { name: ability });
        }
        Ok(Self {
            ability,
            descriptor_version: AbilityDescriptorVersion::new(descriptor_version)?,
            call_mode,
        })
    }

    pub fn default_version(
        ability: impl Into<String>,
        call_mode: CallMode,
    ) -> Result<Self, AbilityControlPlaneError> {
        let ability = ability.into();
        if ability.trim().is_empty() {
            return Err(AbilityControlPlaneError::EmptyDescriptorName);
        }
        if !is_valid_ability_name(&ability) {
            return Err(AbilityControlPlaneError::InvalidDescriptorName { name: ability });
        }
        Ok(Self {
            ability,
            descriptor_version: AbilityDescriptorVersion::default(),
            call_mode,
        })
    }

    pub(crate) fn from_validated_parts(
        ability: impl Into<String>,
        descriptor_version: impl Into<String>,
        call_mode: CallMode,
    ) -> Self {
        Self {
            ability: ability.into(),
            descriptor_version: AbilityDescriptorVersion::from_validated(descriptor_version),
            call_mode,
        }
    }

    pub fn ability(&self) -> &str {
        &self.ability
    }

    pub fn descriptor_version(&self) -> &AbilityDescriptorVersion {
        &self.descriptor_version
    }

    pub fn descriptor_version_str(&self) -> &str {
        self.descriptor_version.as_str()
    }

    pub fn call_mode(&self) -> CallMode {
        self.call_mode
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AbilityControlPlaneKey {
    authority_root: String,
    descriptor: AbilityDescriptorKey,
}

impl AbilityControlPlaneKey {
    pub fn new(
        authority_root: impl Into<String>,
        ability: impl Into<String>,
        descriptor_version: impl Into<String>,
        call_mode: CallMode,
    ) -> Result<Self, AbilityControlPlaneError> {
        let authority_root = authority_root.into();
        if !is_stable_authority_root(&authority_root) {
            return Err(AbilityControlPlaneError::InvalidAuthorityRoot { authority_root });
        }
        Ok(Self {
            authority_root,
            descriptor: AbilityDescriptorKey::new(ability, descriptor_version, call_mode)?,
        })
    }

    pub(crate) fn from_validated_parts(
        authority_root: impl Into<String>,
        ability: impl Into<String>,
        descriptor_version: impl Into<String>,
        call_mode: CallMode,
    ) -> Self {
        Self {
            authority_root: authority_root.into(),
            descriptor: AbilityDescriptorKey::from_validated_parts(
                ability,
                descriptor_version,
                call_mode,
            ),
        }
    }

    pub(crate) fn for_authority(binding: &crate::daemon::ability::AuthorityBinding) -> Self {
        Self::from_validated_parts(
            binding.scope().authority_root().to_string(),
            binding.ability().to_string(),
            binding.descriptor_version().to_string(),
            binding.call_mode(),
        )
    }

    pub(crate) fn for_impl(
        authority_root: impl Into<String>,
        binding: &crate::daemon::ability::AbilityImplBinding,
    ) -> Self {
        Self::from_validated_parts(
            authority_root,
            binding.ability().to_string(),
            binding.descriptor_version().to_string(),
            binding.call_mode(),
        )
    }

    pub fn authority_root(&self) -> &str {
        &self.authority_root
    }

    pub fn ability(&self) -> &str {
        self.descriptor.ability()
    }

    pub fn descriptor_version_str(&self) -> &str {
        self.descriptor.descriptor_version_str()
    }

    pub fn call_mode(&self) -> CallMode {
        self.descriptor.call_mode()
    }
}

fn is_stable_authority_root(authority_root: &str) -> bool {
    authority_root == authority_root.trim()
        && !authority_root.is_empty()
        && !authority_root.chars().any(char::is_whitespace)
        && !authority_root.chars().any(char::is_control)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallMode {
    Rpc,
    Stream,
    Bidi,
}

impl CallMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rpc => "rpc",
            Self::Stream => "stream",
            Self::Bidi => "bidi",
        }
    }

    pub fn axon_call_mode(self) -> axon_sdk::invocation::axiom::AbilityCallMode {
        match self {
            Self::Rpc => axon_sdk::invocation::axiom::AbilityCallMode::Unary,
            Self::Stream => axon_sdk::invocation::axiom::AbilityCallMode::ServerStream,
            Self::Bidi => axon_sdk::invocation::axiom::AbilityCallMode::Bidi,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SchemaHash(pub [u8; 32]);

impl SchemaHash {
    pub fn hex(self) -> String {
        hex::encode(self.0)
    }

    pub fn prefixed_hex(self) -> String {
        format!("sha256:{}", self.hex())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DescriptorHash(pub [u8; 32]);

impl DescriptorHash {
    pub fn hex(self) -> String {
        hex::encode(self.0)
    }

    pub fn prefixed_hex(self) -> String {
        format!("sha256:{}", self.hex())
    }
}

pub(crate) fn is_valid_ability_name(name: &str) -> bool {
    if name.trim().is_empty() || name.trim() != name {
        return false;
    }
    if name.split('.').any(str::is_empty) {
        return false;
    }
    name.bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

pub(crate) fn is_valid_descriptor_version(version: &str) -> bool {
    crate::daemon::ability::manifest::is_valid_descriptor_version(version)
}

#[derive(Debug, Default, Clone)]
pub struct AbilityDescriptorRegistry {
    descriptors: BTreeMap<AbilityControlPlaneKey, AbilityDescriptor>,
}

impl AbilityDescriptorRegistry {
    pub(crate) fn register(&mut self, key: AbilityControlPlaneKey, descriptor: AbilityDescriptor) {
        self.descriptors.insert(key, descriptor);
    }

    pub(crate) fn get(&self, key: &AbilityControlPlaneKey) -> Option<&AbilityDescriptor> {
        self.descriptors.get(key)
    }

    pub(crate) fn keys(&self) -> impl Iterator<Item = &AbilityControlPlaneKey> {
        self.descriptors.keys()
    }

    pub(crate) fn records_for_authority_mode(
        &self,
        authority_root: &str,
        ability: &str,
        call_mode: CallMode,
    ) -> Vec<(AbilityControlPlaneKey, AbilityDescriptor)> {
        self.descriptors
            .iter()
            .filter(|(key, _)| {
                key.authority_root() == authority_root
                    && key.ability() == ability
                    && key.call_mode() == call_mode
            })
            .map(|(key, record)| (key.clone(), record.clone()))
            .collect()
    }

    pub(crate) fn remove_matching(
        &mut self,
        mut predicate: impl FnMut(&AbilityControlPlaneKey) -> bool,
    ) -> bool {
        let before = self.descriptors.len();
        self.descriptors.retain(|key, _| !predicate(key));
        self.descriptors.len() != before
    }

    pub(crate) fn contains_matching(
        &self,
        predicate: impl FnMut(&AbilityControlPlaneKey) -> bool,
    ) -> bool {
        self.descriptors.keys().any(predicate)
    }

    pub(crate) fn names(&self) -> Vec<String> {
        let mut names = self
            .descriptors
            .keys()
            .map(|key| key.ability().to_string())
            .collect::<Vec<_>>();
        names.sort();
        names.dedup();
        names
    }
}

pub(crate) struct GovernedSchemaProjection<'a> {
    pub(crate) input: &'a Value,
    pub(crate) output: &'a Value,
    pub(crate) access_policy: Value,
    pub(crate) hints: Value,
    pub(crate) receipt_semantics: Value,
    pub(crate) admission_action: Value,
    pub(crate) description: &'a str,
    pub(crate) source: &'a str,
    pub(crate) metadata: Value,
}

pub(crate) fn governed_schema_summary(projection: GovernedSchemaProjection<'_>) -> Value {
    serde_json::json!({
        "input": projection.input,
        "output": projection.output,
        "access_policy": projection.access_policy,
        "hints": projection.hints,
        "receipt_semantics": projection.receipt_semantics,
        "admission_action": projection.admission_action,
        "description": projection.description,
        "source": projection.source,
        "metadata": projection.metadata,
    })
}

pub fn governed_access_policy_summary(
    visibility: Value,
    scope_subjects: Value,
    scope_agents: Value,
    deny_callers: Value,
) -> Value {
    serde_json::json!({
        "visibility": visibility,
        "scope_subjects": scope_subjects,
        "scope_agents": scope_agents,
        "deny_callers": deny_callers,
    })
}

pub fn schema_hash_for_governed_summary(summary: &Value) -> SchemaHash {
    let empty = Value::Object(Default::default());
    SchemaHash(
        axon_sdk::invocation::axiom::ability_schema_hash(summary, &empty)
            .expect("Axon ability schema hash must serialize JSON schema values"),
    )
}

pub fn descriptor_hash_for_ability_ura_parts(
    ability_ura: &str,
    name: &str,
    version: &str,
    call_mode: CallMode,
    schema_hash: SchemaHash,
) -> DescriptorHash {
    let descriptor = axon_sdk::invocation::axiom::CanonicalAbilityDescriptor {
        ability_ura: ability_ura.to_string(),
        name: name.to_string(),
        version: version.to_string(),
        call_mode: call_mode.axon_call_mode(),
        schema_hash: schema_hash.0,
        descriptor_hash: [0u8; 32],
    };
    DescriptorHash(axon_sdk::invocation::axiom::ability_descriptor_hash(
        &descriptor,
    ))
}

pub fn canonical_json_bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec(&sort_json_value(value))
        .expect("serde_json::Value serialization cannot fail")
}

fn sort_json_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted = serde_json::Map::new();
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                if let Some(value) = map.get(key) {
                    sorted.insert(key.clone(), sort_json_value(value));
                }
            }
            Value::Object(sorted)
        }
        Value::Array(values) => Value::Array(values.iter().map(sort_json_value).collect()),
        other => other.clone(),
    }
}

pub fn sha256_bytes(bytes: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const LOCAL_SYSTEM_AGENT_URA: &str =
        "easynet:///r/default/agent/device.local.runtime-introspection";

    #[test]
    fn schema_hash_is_stable_under_object_key_order() {
        let a = canonical_json_bytes(&json!({"b": 2, "a": {"d": 4, "c": 3}}));
        let b = canonical_json_bytes(&json!({"a": {"c": 3, "d": 4}, "b": 2}));
        assert_eq!(a, b);
        assert_eq!(sha256_bytes(&a), sha256_bytes(&b));
    }

    #[test]
    fn descriptor_hash_changes_when_version_changes() {
        let descriptor = AbilityDescriptor::new(
            "fs.read",
            LOCAL_SYSTEM_AGENT_URA,
            Visibility::Scoped,
            AdmissionAction::Invoke,
        )
        .unwrap();
        let v1 = descriptor.clone().with_version("1.0.0").unwrap();
        let v2 = descriptor.with_version("2.0.0").unwrap();
        assert_ne!(v1.descriptor_hash_bytes(), v2.descriptor_hash_bytes());
    }

    #[test]
    fn manifest_normalization_has_one_descriptor_hash_path() {
        let manifest = crate::daemon::ability::manifest::AbilityManifest::new(
            "quote",
            "emit a quotable line",
            json!({"type": "object"}),
        )
        .unwrap()
        .with_descriptor_version("2.0.0")
        .unwrap();
        let descriptor = AbilityDescriptor::from_registry_manifest(
            "mentor.quote",
            "easynet:///r/default/agent/u.mentor",
            CallMode::Rpc,
            AdmissionAction::Invoke,
            &manifest,
        )
        .unwrap();
        assert_eq!(descriptor.version, "2.0.0");
        assert_eq!(
            descriptor.descriptor_hash_bytes(),
            descriptor_hash_for_ability_ura_parts(
                &descriptor.canonical_ability_ura().unwrap(),
                &descriptor.public_name(),
                &descriptor.version,
                descriptor.call_mode(),
                SchemaHash(descriptor.schema_hash_bytes()),
            )
            .0
        );
    }

    #[test]
    fn constructors_reject_empty_boundary_values_without_panicking() {
        assert_eq!(
            AbilityDescriptorVersion::new(" ").unwrap_err(),
            AbilityControlPlaneError::EmptyDescriptorVersion
        );
        assert_eq!(
            AbilityDescriptorKey::new("", "1.0.0", CallMode::Rpc).unwrap_err(),
            AbilityControlPlaneError::EmptyDescriptorName
        );
        assert_eq!(
            AbilityDescriptorKey::new("bad/name", "1.0.0", CallMode::Rpc).unwrap_err(),
            AbilityControlPlaneError::InvalidDescriptorName {
                name: "bad/name".to_string()
            }
        );
        for invalid in [
            ".fs.read", "fs..read", "fs.read.", "fs read", "fs:read", "fs?read", "fs{read}",
            "fs/read", "fs\\read",
        ] {
            assert_eq!(
                AbilityDescriptorKey::new(invalid, "1.0.0", CallMode::Rpc).unwrap_err(),
                AbilityControlPlaneError::InvalidDescriptorName {
                    name: invalid.to_string()
                },
                "{invalid:?} must be rejected at the control-plane boundary"
            );
        }
        assert_eq!(
            AbilityDescriptorKey::new("fs.read", "v1", CallMode::Rpc).unwrap_err(),
            AbilityControlPlaneError::InvalidDescriptorVersion {
                version: "v1".to_string()
            }
        );
    }
}
