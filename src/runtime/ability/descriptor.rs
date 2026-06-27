// EasyNet CLI - AbilityDescriptor registry facts
// ==============================================
//
// File: src/runtime/ability/descriptor.rs
// Description: Versioned governed interface facts for daemon-published
//              abilities. These are local control-plane records, not handler
//              implementations.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::AbilityControlPlaneError;

pub const DEFAULT_ABILITY_DESCRIPTOR_VERSION: &str =
    crate::core::ability_spec::DEFAULT_DESCRIPTOR_VERSION;

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

    pub(crate) fn for_descriptor(
        authority_root: impl Into<String>,
        descriptor: &AbilityDescriptorRecord,
    ) -> Self {
        Self::from_validated_parts(
            authority_root,
            descriptor.name.clone(),
            descriptor.version.to_string(),
            descriptor.call_mode,
        )
    }

    pub(crate) fn for_authority(binding: &crate::runtime::ability::AuthorityBindingRecord) -> Self {
        Self::from_validated_parts(
            binding.scope().authority_root().to_string(),
            binding.ability().to_string(),
            binding.descriptor_version().to_string(),
            binding.call_mode(),
        )
    }

    pub(crate) fn for_impl(
        authority_root: impl Into<String>,
        binding: &crate::runtime::ability::AbilityImplBinding,
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

    pub fn axon_call_mode(self) -> easynet_axon::invocation::axiom::AbilityCallMode {
        match self {
            Self::Rpc => easynet_axon::invocation::axiom::AbilityCallMode::Unary,
            Self::Stream => easynet_axon::invocation::axiom::AbilityCallMode::ServerStream,
            Self::Bidi => easynet_axon::invocation::axiom::AbilityCallMode::Bidi,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbilityDescriptorRecord {
    ability_ura: String,
    name: String,
    version: AbilityDescriptorVersion,
    call_mode: CallMode,
    schema_hash: SchemaHash,
    descriptor_hash: DescriptorHash,
}

impl AbilityDescriptorRecord {
    pub fn from_manifest(
        name: impl Into<String>,
        call_mode: CallMode,
        manifest: Option<&crate::core::ability_spec::AbilityManifest>,
    ) -> Result<Self, AbilityControlPlaneError> {
        let version = manifest
            .map(crate::core::ability_spec::AbilityManifest::descriptor_version)
            .unwrap_or(DEFAULT_ABILITY_DESCRIPTOR_VERSION);
        Self::new(name, version, call_mode, manifest)
    }

    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        call_mode: CallMode,
        manifest: Option<&crate::core::ability_spec::AbilityManifest>,
    ) -> Result<Self, AbilityControlPlaneError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(AbilityControlPlaneError::EmptyDescriptorName);
        }
        if !is_valid_ability_name(&name) {
            return Err(AbilityControlPlaneError::InvalidDescriptorName { name });
        }
        let version = AbilityDescriptorVersion::new(version)?;
        ensure_manifest_descriptor_version_matches(version.as_str(), manifest)?;
        let schema_hash = schema_hash_for_manifest(manifest);
        let ability_ura = local_control_plane_ability_ura(&name);
        let descriptor_hash = descriptor_hash_for_ability_ura_parts(
            &ability_ura,
            &name,
            version.as_str(),
            call_mode,
            schema_hash,
        );
        Ok(Self {
            ability_ura,
            name,
            version,
            call_mode,
            schema_hash,
            descriptor_hash,
        })
    }

    pub fn for_ability_ura(
        ability_ura: impl Into<String>,
        name: impl Into<String>,
        version: impl Into<String>,
        call_mode: CallMode,
        manifest: Option<&crate::core::ability_spec::AbilityManifest>,
    ) -> Result<Self, AbilityControlPlaneError> {
        let ability_ura = ability_ura.into();
        validate_descriptor_ability_ura(&ability_ura)?;
        let mut record = Self::new(name, version, call_mode, manifest)?;
        record.ability_ura = ability_ura;
        record.descriptor_hash = descriptor_hash_for_ability_ura_parts(
            &record.ability_ura,
            &record.name,
            record.version.as_str(),
            record.call_mode,
            record.schema_hash,
        );
        Ok(record)
    }

    pub fn to_axon_descriptor(
        &self,
    ) -> easynet_axon::invocation::axiom::CanonicalAbilityDescriptor {
        let mut descriptor = easynet_axon::invocation::axiom::CanonicalAbilityDescriptor {
            ability_ura: self.ability_ura.clone(),
            name: self.name.clone(),
            version: self.version.to_string(),
            call_mode: self.call_mode.axon_call_mode(),
            schema_hash: self.schema_hash.0,
            descriptor_hash: [0u8; 32],
        };
        descriptor.descriptor_hash =
            easynet_axon::invocation::axiom::ability_descriptor_hash(&descriptor);
        descriptor
    }

    pub fn ability_ura(&self) -> &str {
        &self.ability_ura
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn version(&self) -> &AbilityDescriptorVersion {
        &self.version
    }

    pub fn call_mode(&self) -> CallMode {
        self.call_mode
    }

    pub fn schema_hash(&self) -> SchemaHash {
        self.schema_hash
    }

    pub fn descriptor_hash(&self) -> DescriptorHash {
        self.descriptor_hash
    }

    pub fn key(&self) -> AbilityDescriptorKey {
        AbilityDescriptorKey {
            ability: self.name.clone(),
            descriptor_version: self.version.clone(),
            call_mode: self.call_mode,
        }
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
    crate::core::ability_spec::is_valid_descriptor_version(version)
}

fn ensure_manifest_descriptor_version_matches(
    registration_version: &str,
    manifest: Option<&crate::core::ability_spec::AbilityManifest>,
) -> Result<(), AbilityControlPlaneError> {
    let Some(manifest) = manifest else {
        return Ok(());
    };
    let manifest_version = manifest.descriptor_version();
    if manifest_version == registration_version {
        return Ok(());
    }
    Err(AbilityControlPlaneError::DescriptorVersionMismatch {
        manifest_version: manifest_version.to_string(),
        registration_version: registration_version.to_string(),
    })
}

#[derive(Debug, Default, Clone)]
pub struct AbilityDescriptorRegistry {
    descriptors: BTreeMap<AbilityControlPlaneKey, AbilityDescriptorRecord>,
}

impl AbilityDescriptorRegistry {
    pub(crate) fn register(
        &mut self,
        key: AbilityControlPlaneKey,
        descriptor: AbilityDescriptorRecord,
    ) {
        self.descriptors.insert(key, descriptor);
    }

    pub(crate) fn get(&self, key: &AbilityControlPlaneKey) -> Option<&AbilityDescriptorRecord> {
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
    ) -> Vec<(AbilityControlPlaneKey, AbilityDescriptorRecord)> {
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

pub fn schema_hash_for_manifest(
    manifest: Option<&crate::core::ability_spec::AbilityManifest>,
) -> SchemaHash {
    let summary = governed_schema_summary_for_manifest(manifest);
    schema_hash_for_governed_summary(&summary)
}

pub fn governed_schema_summary(
    input: &Value,
    output: &Value,
    access_policy: Value,
    hints: Value,
) -> Value {
    serde_json::json!({
        "input": input,
        "output": output,
        "access_policy": access_policy,
        "hints": hints,
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
        easynet_axon::invocation::axiom::ability_schema_hash(summary, &empty)
            .expect("Axon ability schema hash must serialize JSON schema values"),
    )
}

pub fn schema_hash_for_schema_summary(input: &Value, output: &Value) -> SchemaHash {
    SchemaHash(
        easynet_axon::invocation::axiom::ability_schema_hash(input, output)
            .expect("Axon ability schema hash must serialize JSON schema values"),
    )
}

pub fn descriptor_hash_for_parts(
    name: &str,
    version: &str,
    call_mode: CallMode,
    schema_hash: SchemaHash,
) -> DescriptorHash {
    descriptor_hash_for_ability_ura_parts(
        &local_control_plane_ability_ura(name),
        name,
        version,
        call_mode,
        schema_hash,
    )
}

fn local_control_plane_ability_ura(name: &str) -> String {
    let digest = Sha256::digest(name.as_bytes());
    let local_name = format!("control.{}", hex::encode(digest));
    crate::ura::owner_ability_ura(&crate::ura::hub_ura("local"), &local_name)
        .expect("validated descriptor name must build a local control-plane Ability URA")
}

fn validate_descriptor_ability_ura(ability_ura: &str) -> Result<(), AbilityControlPlaneError> {
    if ability_ura.trim().is_empty() {
        return Err(AbilityControlPlaneError::EmptyDescriptorAbilityUra);
    }
    let parsed = crate::ura::parse_ura(ability_ura).map_err(|_| {
        AbilityControlPlaneError::InvalidDescriptorAbilityUra {
            ability_ura: ability_ura.to_string(),
        }
    })?;
    if parsed.kind != crate::ura::URAKind::Ability {
        return Err(AbilityControlPlaneError::InvalidDescriptorAbilityUra {
            ability_ura: ability_ura.to_string(),
        });
    }
    Ok(())
}

fn governed_schema_summary_for_manifest(
    manifest: Option<&crate::core::ability_spec::AbilityManifest>,
) -> Value {
    let empty = Value::Object(Default::default());
    match manifest {
        Some(manifest) => {
            let access = manifest.access();
            let access_policy = manifest_access_policy_projection(&access);
            governed_schema_summary(
                manifest.input_schema(),
                manifest.output_schema().unwrap_or(&empty),
                access_policy,
                serde_json::to_value(crate::runtime::ability_descriptor::AbilityHints::default())
                    .expect("ability hints serialize"),
            )
        }
        None => {
            let access = crate::core::ability_spec::AccessPolicy::default();
            let access_policy = manifest_access_policy_projection(&access);
            governed_schema_summary(
                &empty,
                &empty,
                access_policy,
                serde_json::to_value(crate::runtime::ability_descriptor::AbilityHints::default())
                    .expect("ability hints serialize"),
            )
        }
    }
}

fn manifest_access_policy_projection(access: &crate::core::ability_spec::AccessPolicy) -> Value {
    governed_access_policy_summary(
        serde_json::to_value(manifest_visibility_projection(access.visibility))
            .expect("manifest visibility projection serializes"),
        serde_json::to_value(crate::runtime::ability_descriptor::ScopeRule::Any)
            .expect("scope rule serializes"),
        serde_json::to_value(manifest_scope_agents_projection(access))
            .expect("scope rule serializes"),
        serde_json::to_value(sorted_policy_list(access.deny_callers.as_deref()))
            .expect("deny caller list serializes"),
    )
}

fn manifest_visibility_projection(
    visibility: crate::core::ability_spec::Visibility,
) -> crate::runtime::ability_descriptor::Visibility {
    match visibility {
        crate::core::ability_spec::Visibility::Selfish => {
            crate::runtime::ability_descriptor::Visibility::Private
        }
        crate::core::ability_spec::Visibility::Device => {
            crate::runtime::ability_descriptor::Visibility::Scoped
        }
        crate::core::ability_spec::Visibility::Public => {
            crate::runtime::ability_descriptor::Visibility::Public
        }
    }
}

fn manifest_scope_agents_projection(
    access: &crate::core::ability_spec::AccessPolicy,
) -> crate::runtime::ability_descriptor::ScopeRule {
    match access.allow_callers.as_ref() {
        Some(allow) if !allow.is_empty() => {
            crate::runtime::ability_descriptor::ScopeRule::OnlyMatching(sorted_policy_list(Some(
                allow.as_slice(),
            )))
        }
        _ => crate::runtime::ability_descriptor::ScopeRule::Any,
    }
}

fn sorted_policy_list(values: Option<&[String]>) -> Vec<String> {
    let mut values = values
        .unwrap_or_default()
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

pub fn descriptor_hash_for_manifest_parts(
    name: &str,
    version: &str,
    call_mode: CallMode,
    schema_hash: SchemaHash,
    manifest: Option<&crate::core::ability_spec::AbilityManifest>,
) -> DescriptorHash {
    let governed_schema_hash = manifest
        .map(|manifest| schema_hash_for_manifest(Some(manifest)))
        .unwrap_or(schema_hash);
    descriptor_hash_for_parts(name, version, call_mode, governed_schema_hash)
}

pub fn descriptor_hash_for_ability_ura_parts(
    ability_ura: &str,
    name: &str,
    version: &str,
    call_mode: CallMode,
    schema_hash: SchemaHash,
) -> DescriptorHash {
    let descriptor = easynet_axon::invocation::axiom::CanonicalAbilityDescriptor {
        ability_ura: ability_ura.to_string(),
        name: name.to_string(),
        version: version.to_string(),
        call_mode: call_mode.axon_call_mode(),
        schema_hash: schema_hash.0,
        descriptor_hash: [0u8; 32],
    };
    DescriptorHash(easynet_axon::invocation::axiom::ability_descriptor_hash(
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

    const LOCAL_DEVICE_URA: &str = "easynet:///r/default/device/local";

    #[test]
    fn schema_hash_is_stable_under_object_key_order() {
        let a = canonical_json_bytes(&json!({"b": 2, "a": {"d": 4, "c": 3}}));
        let b = canonical_json_bytes(&json!({"a": {"c": 3, "d": 4}, "b": 2}));
        assert_eq!(a, b);
        assert_eq!(sha256_bytes(&a), sha256_bytes(&b));
    }

    #[test]
    fn descriptor_hash_changes_when_version_changes() {
        let schema_hash = SchemaHash([7u8; 32]);
        let v1 = descriptor_hash_for_parts("fs.read", "1.0.0", CallMode::Rpc, schema_hash);
        let v2 = descriptor_hash_for_parts("fs.read", "2.0.0", CallMode::Rpc, schema_hash);
        assert_ne!(v1, v2);
    }

    #[test]
    fn descriptor_hash_binds_manifest_access_policy() {
        let input = json!({"type": "object"});
        let base = crate::core::ability_spec::AbilityManifest::new(
            "quote",
            "emit a quotable line",
            input.clone(),
        )
        .unwrap();
        let restricted =
            crate::core::ability_spec::AbilityManifest::new("quote", "emit a quotable line", input)
                .unwrap()
                .with_access(crate::core::ability_spec::AccessPolicy {
                    visibility: crate::core::ability_spec::Visibility::Selfish,
                    allow_callers: None,
                    deny_callers: None,
                })
                .unwrap();

        let base_record =
            AbilityDescriptorRecord::from_manifest("mentor.quote", CallMode::Rpc, Some(&base))
                .unwrap();
        let restricted_record = AbilityDescriptorRecord::from_manifest(
            "mentor.quote",
            CallMode::Rpc,
            Some(&restricted),
        )
        .unwrap();
        assert_ne!(
            base_record.descriptor_hash(),
            restricted_record.descriptor_hash(),
            "descriptor_hash must change when ability access policy changes"
        );
    }

    #[test]
    fn descriptor_hash_binds_manifest_deny_callers() {
        let input = json!({"type": "object"});
        let base = crate::core::ability_spec::AbilityManifest::new(
            "quote",
            "emit a quotable line",
            input.clone(),
        )
        .unwrap()
        .with_access(crate::core::ability_spec::AccessPolicy {
            visibility: crate::core::ability_spec::Visibility::Device,
            allow_callers: Some(vec!["alice".to_string()]),
            deny_callers: None,
        })
        .unwrap();
        let deny_alice =
            crate::core::ability_spec::AbilityManifest::new("quote", "emit a quotable line", input)
                .unwrap()
                .with_access(crate::core::ability_spec::AccessPolicy {
                    visibility: crate::core::ability_spec::Visibility::Device,
                    allow_callers: Some(vec!["alice".to_string()]),
                    deny_callers: Some(vec!["alice".to_string()]),
                })
                .unwrap();

        let base_record =
            AbilityDescriptorRecord::from_manifest("mentor.quote", CallMode::Rpc, Some(&base))
                .unwrap();
        let deny_record = AbilityDescriptorRecord::from_manifest(
            "mentor.quote",
            CallMode::Rpc,
            Some(&deny_alice),
        )
        .unwrap();
        assert_ne!(
            base_record.descriptor_hash(),
            deny_record.descriptor_hash(),
            "descriptor_hash must bind deny_callers because deny overrides allow at invoke time"
        );
    }

    #[test]
    fn manifest_schema_hash_matches_runtime_descriptor_projection() {
        let input = json!({
            "type": "object",
            "properties": {"topic": {"type": "string"}},
            "required": ["topic"],
        });
        let output = json!({
            "type": "object",
            "properties": {"quote": {"type": "string"}},
            "required": ["quote"],
        });
        let manifest = crate::core::ability_spec::AbilityManifest::new(
            "quote",
            "emit a quotable line",
            input.clone(),
        )
        .unwrap()
        .with_output_schema(output.clone())
        .unwrap()
        .with_access(crate::core::ability_spec::AccessPolicy {
            visibility: crate::core::ability_spec::Visibility::Device,
            allow_callers: Some(vec!["alice".to_string(), "bob".to_string()]),
            deny_callers: None,
        })
        .unwrap();

        let control_plane_record =
            AbilityDescriptorRecord::from_manifest("mentor.quote", CallMode::Rpc, Some(&manifest))
                .unwrap();
        let runtime_descriptor = crate::runtime::ability_descriptor::AbilityDescriptor::new(
            "mentor.quote",
            LOCAL_DEVICE_URA,
            crate::runtime::ability_descriptor::Visibility::Scoped,
        )
        .unwrap()
        .with_input_schema(input)
        .with_output_schema(output)
        .with_scope_agents(crate::runtime::ability_descriptor::ScopeRule::OnlyMatching(
            vec!["alice".to_string(), "bob".to_string()],
        ));

        assert_eq!(
            control_plane_record.schema_hash().0,
            runtime_descriptor.schema_hash_bytes(),
            "manifest-derived control-plane schema hashes must use the same governed summary as runtime AbilityDescriptor"
        );
    }

    #[test]
    fn descriptor_record_from_manifest_uses_manifest_descriptor_version() {
        let manifest = crate::core::ability_spec::AbilityManifest::new(
            "quote",
            "emit a quotable line",
            json!({"type": "object"}),
        )
        .unwrap()
        .with_descriptor_version("2.0.0")
        .unwrap();

        let record =
            AbilityDescriptorRecord::from_manifest("mentor.quote", CallMode::Rpc, Some(&manifest))
                .unwrap();
        assert_eq!(record.version().as_str(), "2.0.0");
    }

    #[test]
    fn descriptor_record_rejects_explicit_version_that_disagrees_with_manifest() {
        let manifest = crate::core::ability_spec::AbilityManifest::new(
            "quote",
            "emit a quotable line",
            json!({"type": "object"}),
        )
        .unwrap()
        .with_descriptor_version("2.0.0")
        .unwrap();

        let err =
            AbilityDescriptorRecord::new("mentor.quote", "1.0.0", CallMode::Rpc, Some(&manifest))
                .unwrap_err();
        assert_eq!(
            err,
            AbilityControlPlaneError::DescriptorVersionMismatch {
                manifest_version: "2.0.0".to_string(),
                registration_version: "1.0.0".to_string(),
            }
        );
    }

    #[test]
    fn constructors_reject_empty_boundary_values_without_panicking() {
        assert_eq!(
            AbilityDescriptorVersion::new(" ").unwrap_err(),
            AbilityControlPlaneError::EmptyDescriptorVersion
        );
        assert_eq!(
            AbilityDescriptorRecord::from_manifest("", CallMode::Rpc, None).unwrap_err(),
            AbilityControlPlaneError::EmptyDescriptorName
        );
        assert_eq!(
            AbilityDescriptorRecord::from_manifest("bad/name", CallMode::Rpc, None).unwrap_err(),
            AbilityControlPlaneError::InvalidDescriptorName {
                name: "bad/name".to_string()
            }
        );
        for invalid in [
            ".fs.read", "fs..read", "fs.read.", "fs read", "fs:read", "fs?read", "fs{read}",
            "fs/read", "fs\\read",
        ] {
            assert_eq!(
                AbilityDescriptorRecord::from_manifest(invalid, CallMode::Rpc, None).unwrap_err(),
                AbilityControlPlaneError::InvalidDescriptorName {
                    name: invalid.to_string()
                },
                "{invalid:?} must be rejected at the control-plane boundary"
            );
        }
        assert_eq!(
            AbilityDescriptorRecord::new("fs.read", "v1", CallMode::Rpc, None).unwrap_err(),
            AbilityControlPlaneError::InvalidDescriptorVersion {
                version: "v1".to_string()
            }
        );
    }
}
