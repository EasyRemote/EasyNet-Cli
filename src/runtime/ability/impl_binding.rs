// EasyNet CLI - AbilityImpl binding registry facts
// ================================================
//
// File: src/runtime/ability/impl_binding.rs
// Description: Versioned executable binding records for local ability
//              implementations. Plugin/native/EAL/MCP side effects live here,
//              not in AbilityDescriptor.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::descriptor::{
    canonical_json_bytes, is_valid_ability_name, is_valid_descriptor_version, sha256_bytes,
    unique_record_for, AbilityDescriptorKey, CallMode,
};
use super::AbilityControlPlaneError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeEnv {
    label: String,
}

impl RuntimeEnv {
    pub fn new(label: impl Into<String>) -> Result<Self, AbilityControlPlaneError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(AbilityControlPlaneError::EmptyRuntimeEnv);
        }
        Ok(Self { label })
    }

    fn generated(label: String) -> Self {
        Self { label }
    }

    pub fn daemon_native() -> Self {
        Self::generated(format!(
            "easynet-cli/{};rust-native",
            env!("CARGO_PKG_VERSION")
        ))
    }

    pub fn plugin_sidecar(package: &str, version: &str) -> Self {
        Self::generated(format!(
            "easynet-cli/{};plugin:{package}@{version}",
            env!("CARGO_PKG_VERSION")
        ))
    }

    pub fn mcp(server: &str) -> Self {
        Self::generated(format!(
            "easynet-cli/{};mcp:{server}",
            env!("CARGO_PKG_VERSION")
        ))
    }

    pub fn device_ability(exec_kind: &str) -> Self {
        Self::generated(format!(
            "easynet-cli/{};device-ability:{exec_kind}",
            env!("CARGO_PKG_VERSION")
        ))
    }

    pub fn label(&self) -> &str {
        &self.label
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbilityImplSource {
    NativeDaemon,
    BuiltinPlugin,
    SidecarPlugin,
    DeclarativePlugin,
    DeviceDeploy,
    Eal,
    Mcp,
    Test,
}

impl AbilityImplSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NativeDaemon => "native_daemon",
            Self::BuiltinPlugin => "builtin_plugin",
            Self::SidecarPlugin => "sidecar_plugin",
            Self::DeclarativePlugin => "declarative_plugin",
            Self::DeviceDeploy => "device_deploy",
            Self::Eal => "eal",
            Self::Mcp => "mcp",
            Self::Test => "test",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbilityImplBinding {
    ability: String,
    descriptor_version: String,
    call_mode: CallMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content_hash: Option<String>,
    impl_hash: [u8; 32],
    runtime_env: RuntimeEnv,
    source: AbilityImplSource,
}

impl AbilityImplBinding {
    pub fn new(
        ability: impl Into<String>,
        descriptor_version: impl Into<String>,
        call_mode: CallMode,
        runtime_env: RuntimeEnv,
        source: AbilityImplSource,
    ) -> Result<Self, AbilityControlPlaneError> {
        Self::new_with_content_hash(
            ability,
            descriptor_version,
            call_mode,
            runtime_env,
            source,
            None,
        )
    }

    pub fn new_with_content_hash(
        ability: impl Into<String>,
        descriptor_version: impl Into<String>,
        call_mode: CallMode,
        runtime_env: RuntimeEnv,
        source: AbilityImplSource,
        content_hash: Option<String>,
    ) -> Result<Self, AbilityControlPlaneError> {
        let ability = ability.into();
        let descriptor_version = descriptor_version.into();
        if ability.trim().is_empty() {
            return Err(AbilityControlPlaneError::EmptyImplementationAbility);
        }
        if !is_valid_ability_name(&ability) {
            return Err(AbilityControlPlaneError::InvalidImplementationAbility { ability });
        }
        if descriptor_version.trim().is_empty() {
            return Err(AbilityControlPlaneError::EmptyImplementationDescriptorVersion);
        }
        if !is_valid_descriptor_version(&descriptor_version) {
            return Err(
                AbilityControlPlaneError::InvalidImplementationDescriptorVersion {
                    version: descriptor_version,
                },
            );
        }
        if let Some(hash) = content_hash.as_deref() {
            if !is_valid_sha256_content_hash(hash) {
                return Err(AbilityControlPlaneError::InvalidImplementationContentHash {
                    hash: hash.to_string(),
                });
            }
        }
        let impl_hash = impl_hash_for_parts(
            &ability,
            &descriptor_version,
            call_mode,
            &runtime_env,
            &source,
            content_hash.as_deref(),
        );
        Ok(Self {
            ability,
            descriptor_version,
            call_mode,
            content_hash,
            impl_hash,
            runtime_env,
            source,
        })
    }

    pub fn impl_hash_hex(&self) -> String {
        hex::encode(self.impl_hash)
    }

    pub fn impl_hash_prefixed_hex(&self) -> String {
        format!("sha256:{}", self.impl_hash_hex())
    }

    pub fn ability(&self) -> &str {
        &self.ability
    }

    pub fn descriptor_version(&self) -> &str {
        &self.descriptor_version
    }

    pub fn call_mode(&self) -> CallMode {
        self.call_mode
    }

    pub fn content_hash(&self) -> Option<&str> {
        self.content_hash.as_deref()
    }

    pub fn impl_hash(&self) -> [u8; 32] {
        self.impl_hash
    }

    pub fn runtime_env(&self) -> &RuntimeEnv {
        &self.runtime_env
    }

    pub fn source(&self) -> &AbilityImplSource {
        &self.source
    }

    pub fn key(&self) -> AbilityDescriptorKey {
        AbilityDescriptorKey::from_validated_parts(
            self.ability.clone(),
            self.descriptor_version.clone(),
            self.call_mode,
        )
    }
}

#[derive(Debug, Default, Clone)]
pub struct AbilityImplRegistry {
    bindings: BTreeMap<AbilityDescriptorKey, AbilityImplBinding>,
}

impl AbilityImplRegistry {
    pub fn bind(&mut self, binding: AbilityImplBinding) {
        self.bindings.insert(binding.key(), binding);
    }

    pub fn remove(&mut self, ability: &str) -> bool {
        let before = self.bindings.len();
        self.bindings.retain(|key, _| key.ability() != ability);
        self.bindings.len() != before
    }

    pub fn get(&self, ability: &str) -> Option<&AbilityImplBinding> {
        self.get_version(ability, super::DEFAULT_ABILITY_DESCRIPTOR_VERSION)
    }

    pub fn get_for_mode(&self, ability: &str, call_mode: CallMode) -> Option<&AbilityImplBinding> {
        self.get_version_for_mode(
            ability,
            super::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
            call_mode,
        )
    }

    pub fn get_version(
        &self,
        ability: &str,
        descriptor_version: &str,
    ) -> Option<&AbilityImplBinding> {
        unique_record_for(&self.bindings, ability, descriptor_version)
    }

    pub fn get_version_for_mode(
        &self,
        ability: &str,
        descriptor_version: &str,
        call_mode: CallMode,
    ) -> Option<&AbilityImplBinding> {
        let key = AbilityDescriptorKey::new(ability, descriptor_version, call_mode).ok()?;
        self.bindings.get(&key)
    }

    pub fn contains(&self, ability: &str) -> bool {
        self.bindings.keys().any(|key| key.ability() == ability)
    }
}

fn is_valid_sha256_content_hash(hash: &str) -> bool {
    let Some(hex) = hash.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64 && hex.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
}

fn impl_hash_for_parts(
    ability: &str,
    descriptor_version: &str,
    call_mode: CallMode,
    runtime_env: &RuntimeEnv,
    source: &AbilityImplSource,
    content_hash: Option<&str>,
) -> [u8; 32] {
    let payload = serde_json::json!({
        "ability": ability,
        "descriptor_version": descriptor_version,
        "call_mode": call_mode.as_str(),
        "content_hash": content_hash,
        "runtime_env": runtime_env.label(),
        "source": source.as_str(),
    });
    sha256_bytes(&canonical_json_bytes(&payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impl_hash_binds_runtime_env() {
        let a = AbilityImplBinding::new(
            "fs.read",
            "1.0.0",
            CallMode::Rpc,
            RuntimeEnv::new("env:a").unwrap(),
            AbilityImplSource::NativeDaemon,
        )
        .unwrap();
        let b = AbilityImplBinding::new(
            "fs.read",
            "1.0.0",
            CallMode::Rpc,
            RuntimeEnv::new("env:b").unwrap(),
            AbilityImplSource::NativeDaemon,
        )
        .unwrap();
        assert_ne!(a.impl_hash(), b.impl_hash());
    }

    #[test]
    fn impl_hash_binds_content_hash_when_present() {
        let a = AbilityImplBinding::new_with_content_hash(
            "er.generate",
            "1.0.0",
            CallMode::Stream,
            RuntimeEnv::device_ability("host_stream"),
            AbilityImplSource::DeviceDeploy,
            Some(format!("sha256:{}", "a".repeat(64))),
        )
        .unwrap();
        let b = AbilityImplBinding::new_with_content_hash(
            "er.generate",
            "1.0.0",
            CallMode::Stream,
            RuntimeEnv::device_ability("host_stream"),
            AbilityImplSource::DeviceDeploy,
            Some(format!("sha256:{}", "b".repeat(64))),
        )
        .unwrap();
        assert_ne!(a.impl_hash(), b.impl_hash());
    }

    #[test]
    fn constructors_reject_empty_boundary_values_without_panicking() {
        assert_eq!(
            RuntimeEnv::new(" ").unwrap_err(),
            AbilityControlPlaneError::EmptyRuntimeEnv
        );
        assert_eq!(
            AbilityImplBinding::new(
                "",
                "1.0.0",
                CallMode::Rpc,
                RuntimeEnv::daemon_native(),
                AbilityImplSource::NativeDaemon,
            )
            .unwrap_err(),
            AbilityControlPlaneError::EmptyImplementationAbility
        );
        assert_eq!(
            AbilityImplBinding::new(
                "bad/name",
                "1.0.0",
                CallMode::Rpc,
                RuntimeEnv::daemon_native(),
                AbilityImplSource::NativeDaemon,
            )
            .unwrap_err(),
            AbilityControlPlaneError::InvalidImplementationAbility {
                ability: "bad/name".to_string()
            }
        );
        assert_eq!(
            AbilityImplBinding::new(
                "fs.read",
                "v1",
                CallMode::Rpc,
                RuntimeEnv::daemon_native(),
                AbilityImplSource::NativeDaemon,
            )
            .unwrap_err(),
            AbilityControlPlaneError::InvalidImplementationDescriptorVersion {
                version: "v1".to_string()
            }
        );
        assert_eq!(
            AbilityImplBinding::new_with_content_hash(
                "fs.read",
                "1.0.0",
                CallMode::Rpc,
                RuntimeEnv::daemon_native(),
                AbilityImplSource::NativeDaemon,
                Some("sha256:abc".to_string()),
            )
            .unwrap_err(),
            AbilityControlPlaneError::InvalidImplementationContentHash {
                hash: "sha256:abc".to_string()
            }
        );
    }
}
