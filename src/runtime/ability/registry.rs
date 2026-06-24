// EasyNet CLI - Ability control-plane registry
// ============================================
//
// File: src/runtime/ability/registry.rs
// Description: Aggregates descriptor, authority, and implementation binding
//              registries for daemon-local ability registration.

use std::collections::BTreeMap;
use std::fmt;

use super::{
    AbilityControlPlaneError, AbilityDescriptorRecord, AbilityImplBinding, AbilityImplSource,
    AuthorityBindingRecord, AuthorityScope, CallMode, RuntimeEnv,
    DEFAULT_ABILITY_DESCRIPTOR_VERSION,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbilityControlPlaneRecord {
    descriptor: AbilityDescriptorRecord,
    authority: AuthorityBindingRecord,
    implementation: AbilityImplBinding,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbilityControlPlaneLookupMatch {
    pub authority_root: String,
    pub call_mode: CallMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbilityControlPlaneLookupError {
    pub ability: String,
    pub descriptor_version: String,
    pub matches: Vec<AbilityControlPlaneLookupMatch>,
}

impl fmt::Display for AbilityControlPlaneLookupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ability {:?} descriptor version {:?} is ambiguous across {} control-plane records",
            self.ability,
            self.descriptor_version,
            self.matches.len()
        )
    }
}

impl std::error::Error for AbilityControlPlaneLookupError {}

impl AbilityControlPlaneRecord {
    pub fn descriptor(&self) -> &AbilityDescriptorRecord {
        &self.descriptor
    }

    pub fn authority(&self) -> &AuthorityBindingRecord {
        &self.authority
    }

    pub fn implementation(&self) -> &AbilityImplBinding {
        &self.implementation
    }
}

#[derive(Debug, Default, Clone)]
pub struct AbilityControlPlaneRegistry {
    records: BTreeMap<AbilityControlPlaneRecordKey, AbilityControlPlaneRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct AbilityControlPlaneRecordKey {
    authority_root: String,
    ability: String,
    descriptor_version: String,
    call_mode: CallMode,
}

impl AbilityControlPlaneRecordKey {
    fn new(
        authority_root: impl Into<String>,
        ability: impl Into<String>,
        descriptor_version: impl Into<String>,
        call_mode: CallMode,
    ) -> Self {
        Self {
            authority_root: authority_root.into(),
            ability: ability.into(),
            descriptor_version: descriptor_version.into(),
            call_mode,
        }
    }

    fn from_record(record: &AbilityControlPlaneRecord) -> Self {
        Self {
            authority_root: record.authority().scope().authority_root().to_string(),
            ability: record.descriptor().name().to_string(),
            descriptor_version: record.descriptor().version().to_string(),
            call_mode: record.descriptor().call_mode(),
        }
    }

    fn ability(&self) -> &str {
        &self.ability
    }

    fn descriptor_version(&self) -> &str {
        &self.descriptor_version
    }

    fn call_mode(&self) -> CallMode {
        self.call_mode
    }

    fn lookup_match(&self) -> AbilityControlPlaneLookupMatch {
        AbilityControlPlaneLookupMatch {
            authority_root: self.authority_root.clone(),
            call_mode: self.call_mode,
        }
    }
}

impl AbilityControlPlaneRegistry {
    pub fn register(
        &mut self,
        ability: impl Into<String>,
        call_mode: CallMode,
        manifest: Option<&crate::core::ability_spec::AbilityManifest>,
        authority_scope: AuthorityScope,
        runtime_env: RuntimeEnv,
        impl_source: AbilityImplSource,
    ) -> Result<AbilityControlPlaneRecord, AbilityControlPlaneError> {
        self.register_version_with_impl_content_hash(
            ability,
            DEFAULT_ABILITY_DESCRIPTOR_VERSION,
            call_mode,
            manifest,
            authority_scope,
            runtime_env,
            impl_source,
            None,
        )
    }

    pub fn register_with_impl_content_hash(
        &mut self,
        ability: impl Into<String>,
        call_mode: CallMode,
        manifest: Option<&crate::core::ability_spec::AbilityManifest>,
        authority_scope: AuthorityScope,
        runtime_env: RuntimeEnv,
        impl_source: AbilityImplSource,
        impl_content_hash: Option<String>,
    ) -> Result<AbilityControlPlaneRecord, AbilityControlPlaneError> {
        self.register_version_with_impl_content_hash(
            ability,
            DEFAULT_ABILITY_DESCRIPTOR_VERSION,
            call_mode,
            manifest,
            authority_scope,
            runtime_env,
            impl_source,
            impl_content_hash,
        )
    }

    pub fn register_version(
        &mut self,
        ability: impl Into<String>,
        descriptor_version: impl Into<String>,
        call_mode: CallMode,
        manifest: Option<&crate::core::ability_spec::AbilityManifest>,
        authority_scope: AuthorityScope,
        runtime_env: RuntimeEnv,
        impl_source: AbilityImplSource,
    ) -> Result<AbilityControlPlaneRecord, AbilityControlPlaneError> {
        self.register_version_with_impl_content_hash(
            ability,
            descriptor_version,
            call_mode,
            manifest,
            authority_scope,
            runtime_env,
            impl_source,
            None,
        )
    }

    pub fn register_version_with_impl_content_hash(
        &mut self,
        ability: impl Into<String>,
        descriptor_version: impl Into<String>,
        call_mode: CallMode,
        manifest: Option<&crate::core::ability_spec::AbilityManifest>,
        authority_scope: AuthorityScope,
        runtime_env: RuntimeEnv,
        impl_source: AbilityImplSource,
        impl_content_hash: Option<String>,
    ) -> Result<AbilityControlPlaneRecord, AbilityControlPlaneError> {
        let ability = ability.into();
        let descriptor_version = descriptor_version.into();
        let ability_ura = crate::ura::owner_ability_ura(authority_scope.authority_root(), &ability)
            .ok_or_else(
                || AbilityControlPlaneError::DescriptorAbilityUraDerivationFailed {
                    authority_root: authority_scope.authority_root().to_string(),
                    ability: ability.clone(),
                },
            )?;
        let descriptor = AbilityDescriptorRecord::for_ability_ura(
            ability_ura,
            &ability,
            descriptor_version,
            call_mode,
            manifest,
        )?;
        let authority = AuthorityBindingRecord::local_self_with_manifest_policy(
            ability.clone(),
            descriptor.version().to_string(),
            call_mode,
            authority_scope,
            manifest,
        )?;
        let implementation = AbilityImplBinding::new_with_content_hash(
            ability,
            descriptor.version().to_string(),
            call_mode,
            runtime_env,
            impl_source,
            impl_content_hash,
        )?;
        let record = AbilityControlPlaneRecord {
            descriptor,
            authority,
            implementation,
        };
        self.records.insert(
            AbilityControlPlaneRecordKey::from_record(&record),
            record.clone(),
        );
        Ok(record)
    }

    pub fn remove(&mut self, ability: &str) -> bool {
        let before = self.records.len();
        self.records.retain(|key, _| key.ability() != ability);
        self.records.len() != before
    }

    pub fn remove_for_mode(&mut self, ability: &str, call_mode: CallMode) -> bool {
        let before = self.records.len();
        self.records
            .retain(|key, _| !(key.ability() == ability && key.call_mode() == call_mode));
        self.records.len() != before
    }

    pub fn remove_for_authority_mode(
        &mut self,
        authority_root: &str,
        ability: &str,
        call_mode: CallMode,
    ) -> bool {
        let key = AbilityControlPlaneRecordKey::new(
            authority_root,
            ability,
            DEFAULT_ABILITY_DESCRIPTOR_VERSION,
            call_mode,
        );
        self.records.remove(&key).is_some()
    }

    pub fn get(
        &self,
        ability: &str,
    ) -> Result<Option<AbilityControlPlaneRecord>, AbilityControlPlaneLookupError> {
        self.get_version(ability, super::DEFAULT_ABILITY_DESCRIPTOR_VERSION)
    }

    pub fn get_for_mode(
        &self,
        ability: &str,
        call_mode: CallMode,
    ) -> Result<Option<AbilityControlPlaneRecord>, AbilityControlPlaneLookupError> {
        self.get_version_for_mode(
            ability,
            super::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
            call_mode,
        )
    }

    pub fn get_for_authority_mode(
        &self,
        authority_root: &str,
        ability: &str,
        call_mode: CallMode,
    ) -> Option<AbilityControlPlaneRecord> {
        self.get_version_for_authority_mode(
            authority_root,
            ability,
            super::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
            call_mode,
        )
    }

    pub fn get_version(
        &self,
        ability: &str,
        descriptor_version: &str,
    ) -> Result<Option<AbilityControlPlaneRecord>, AbilityControlPlaneLookupError> {
        unique_record_for_control_plane(&self.records, ability, descriptor_version, None)
            .map(|r| r.cloned())
    }

    pub fn get_version_for_mode(
        &self,
        ability: &str,
        descriptor_version: &str,
        call_mode: CallMode,
    ) -> Result<Option<AbilityControlPlaneRecord>, AbilityControlPlaneLookupError> {
        unique_record_for_control_plane(&self.records, ability, descriptor_version, Some(call_mode))
            .map(|record| record.cloned())
    }

    pub fn get_version_for_authority_mode(
        &self,
        authority_root: &str,
        ability: &str,
        descriptor_version: &str,
        call_mode: CallMode,
    ) -> Option<AbilityControlPlaneRecord> {
        self.records
            .get(&AbilityControlPlaneRecordKey::new(
                authority_root,
                ability,
                descriptor_version,
                call_mode,
            ))
            .cloned()
    }

    pub fn descriptor(
        &self,
        ability: &str,
    ) -> Result<Option<&AbilityDescriptorRecord>, AbilityControlPlaneLookupError> {
        self.record(ability)
            .map(|record| record.map(AbilityControlPlaneRecord::descriptor))
    }

    pub fn descriptor_for_mode(
        &self,
        ability: &str,
        call_mode: CallMode,
    ) -> Result<Option<&AbilityDescriptorRecord>, AbilityControlPlaneLookupError> {
        self.record_for_mode(ability, call_mode)
            .map(|record| record.map(AbilityControlPlaneRecord::descriptor))
    }

    pub fn authority(
        &self,
        ability: &str,
    ) -> Result<Option<&AuthorityBindingRecord>, AbilityControlPlaneLookupError> {
        self.record(ability)
            .map(|record| record.map(AbilityControlPlaneRecord::authority))
    }

    pub fn authority_for_mode(
        &self,
        ability: &str,
        call_mode: CallMode,
    ) -> Result<Option<&AuthorityBindingRecord>, AbilityControlPlaneLookupError> {
        self.record_for_mode(ability, call_mode)
            .map(|record| record.map(AbilityControlPlaneRecord::authority))
    }

    pub fn implementation(
        &self,
        ability: &str,
    ) -> Result<Option<&AbilityImplBinding>, AbilityControlPlaneLookupError> {
        self.record(ability)
            .map(|record| record.map(AbilityControlPlaneRecord::implementation))
    }

    pub fn implementation_for_mode(
        &self,
        ability: &str,
        call_mode: CallMode,
    ) -> Result<Option<&AbilityImplBinding>, AbilityControlPlaneLookupError> {
        self.record_for_mode(ability, call_mode)
            .map(|record| record.map(AbilityControlPlaneRecord::implementation))
    }

    pub fn contains(&self, ability: &str) -> bool {
        self.records.keys().any(|key| key.ability() == ability)
    }

    pub fn names(&self) -> Vec<String> {
        let mut names = self
            .records
            .keys()
            .map(|key| key.ability().to_string())
            .collect::<Vec<_>>();
        names.sort();
        names.dedup();
        names
    }

    /// Distinct authority roots that own `ability` across all registered modes.
    ///
    /// Runtime dispatch keys are descriptor-owner URAs, not bare ability names.
    /// Callers that need to register or invoke through `LocalRuntime` use this
    /// to prove that a name maps to one owner before deriving a protocol key.
    pub fn authority_roots_for_ability(&self, ability: &str) -> Vec<String> {
        let mut roots = self
            .records
            .keys()
            .filter(|key| key.ability() == ability)
            .map(|key| key.authority_root.clone())
            .collect::<Vec<_>>();
        roots.sort();
        roots.dedup();
        roots
    }

    fn record(
        &self,
        ability: &str,
    ) -> Result<Option<&AbilityControlPlaneRecord>, AbilityControlPlaneLookupError> {
        self.record_version(ability, DEFAULT_ABILITY_DESCRIPTOR_VERSION)
    }

    fn record_for_mode(
        &self,
        ability: &str,
        call_mode: CallMode,
    ) -> Result<Option<&AbilityControlPlaneRecord>, AbilityControlPlaneLookupError> {
        self.record_version_for_mode(ability, DEFAULT_ABILITY_DESCRIPTOR_VERSION, call_mode)
    }

    fn record_version(
        &self,
        ability: &str,
        descriptor_version: &str,
    ) -> Result<Option<&AbilityControlPlaneRecord>, AbilityControlPlaneLookupError> {
        unique_record_for_control_plane(&self.records, ability, descriptor_version, None)
    }

    fn record_version_for_mode(
        &self,
        ability: &str,
        descriptor_version: &str,
        call_mode: CallMode,
    ) -> Result<Option<&AbilityControlPlaneRecord>, AbilityControlPlaneLookupError> {
        unique_record_for_control_plane(&self.records, ability, descriptor_version, Some(call_mode))
    }
}

fn unique_record_for_control_plane<'a>(
    records: &'a BTreeMap<AbilityControlPlaneRecordKey, AbilityControlPlaneRecord>,
    ability: &str,
    descriptor_version: &str,
    call_mode: Option<CallMode>,
) -> Result<Option<&'a AbilityControlPlaneRecord>, AbilityControlPlaneLookupError> {
    let matches = records
        .iter()
        .filter(|(key, _)| {
            key.ability() == ability
                && key.descriptor_version() == descriptor_version
                && call_mode.map_or(true, |mode| key.call_mode() == mode)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Ok(None),
        [(_, record)] => Ok(Some(record)),
        _ => Err(AbilityControlPlaneLookupError {
            ability: ability.to_string(),
            descriptor_version: descriptor_version.to_string(),
            matches: matches.iter().map(|(key, _)| key.lookup_match()).collect(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCAL_DEVICE_URA: &str = "easynet:///r/default/device/local";
    const LOCAL_AGENT_URA: &str = "easynet:///r/default/agent/user.assistant";

    #[test]
    fn register_writes_descriptor_authority_and_impl() {
        let mut registry = AbilityControlPlaneRegistry::default();
        let record = registry
            .register(
                "fs.read",
                CallMode::Rpc,
                None,
                AuthorityScope::new("device", LOCAL_DEVICE_URA).unwrap(),
                RuntimeEnv::daemon_native(),
                AbilityImplSource::NativeDaemon,
            )
            .unwrap();
        assert_eq!(record.descriptor().version().as_str(), "1.0.0");
        assert!(record.authority().predicate().governs_advertise());
        assert!(record.authority().predicate().governs_invoke());
        assert_eq!(
            record.implementation().runtime_env().label(),
            RuntimeEnv::daemon_native().label()
        );
        assert!(registry
            .get("fs.read")
            .expect("single record lookup is unambiguous")
            .is_some());
    }

    #[test]
    fn register_version_keeps_same_ability_versions_distinct() {
        let mut registry = AbilityControlPlaneRegistry::default();
        registry
            .register_version(
                "fs.read",
                "1.0.0",
                CallMode::Rpc,
                None,
                AuthorityScope::new("device", LOCAL_DEVICE_URA).unwrap(),
                RuntimeEnv::new("env:v1").unwrap(),
                AbilityImplSource::NativeDaemon,
            )
            .unwrap();
        registry
            .register_version(
                "fs.read",
                "2.0.0",
                CallMode::Rpc,
                None,
                AuthorityScope::new("device", LOCAL_DEVICE_URA).unwrap(),
                RuntimeEnv::new("env:v2").unwrap(),
                AbilityImplSource::NativeDaemon,
            )
            .unwrap();

        let v1 = registry
            .get_version("fs.read", "1.0.0")
            .expect("v1 lookup is unambiguous")
            .expect("v1 record");
        let v2 = registry
            .get_version("fs.read", "2.0.0")
            .expect("v2 lookup is unambiguous")
            .expect("v2 record");
        assert_eq!(v1.implementation().runtime_env().label(), "env:v1");
        assert_eq!(v2.implementation().runtime_env().label(), "env:v2");
        assert_ne!(
            v1.implementation().impl_hash(),
            v2.implementation().impl_hash()
        );
    }

    #[test]
    fn register_keeps_same_ability_version_modes_distinct() {
        let mut registry = AbilityControlPlaneRegistry::default();
        registry
            .register(
                "agent.chat",
                CallMode::Rpc,
                None,
                AuthorityScope::new("agent:assistant", LOCAL_AGENT_URA).unwrap(),
                RuntimeEnv::new("env:rpc").unwrap(),
                AbilityImplSource::NativeDaemon,
            )
            .unwrap();
        registry
            .register(
                "agent.chat",
                CallMode::Stream,
                None,
                AuthorityScope::new("agent:assistant", LOCAL_AGENT_URA).unwrap(),
                RuntimeEnv::new("env:stream").unwrap(),
                AbilityImplSource::NativeDaemon,
            )
            .unwrap();

        let rpc = registry
            .get_for_mode("agent.chat", CallMode::Rpc)
            .expect("rpc mode lookup is unambiguous")
            .expect("rpc mode record");
        let stream = registry
            .get_for_mode("agent.chat", CallMode::Stream)
            .expect("stream mode lookup is unambiguous")
            .expect("stream mode record");
        assert_eq!(rpc.descriptor().call_mode(), CallMode::Rpc);
        assert_eq!(stream.descriptor().call_mode(), CallMode::Stream);
        assert_eq!(rpc.implementation().runtime_env().label(), "env:rpc");
        assert_eq!(stream.implementation().runtime_env().label(), "env:stream");
        let err = registry
            .get("agent.chat")
            .expect_err("mode-agnostic lookup must not pick an arbitrary record");
        assert_eq!(err.matches.len(), 2);
    }

    #[test]
    fn register_keeps_same_public_name_distinct_across_authority_roots() {
        let mut registry = AbilityControlPlaneRegistry::default();
        registry
            .register(
                "search",
                CallMode::Rpc,
                None,
                AuthorityScope::new("agent:a", "easynet:///r/default/agent/user.a").unwrap(),
                RuntimeEnv::new("env:a").unwrap(),
                AbilityImplSource::NativeDaemon,
            )
            .unwrap();
        registry
            .register(
                "search",
                CallMode::Rpc,
                None,
                AuthorityScope::new("agent:b", "easynet:///r/default/agent/user.b").unwrap(),
                RuntimeEnv::new("env:b").unwrap(),
                AbilityImplSource::NativeDaemon,
            )
            .unwrap();

        let err = registry
            .get_for_mode("search", CallMode::Rpc)
            .expect_err("name-only lookup must not choose between two authority roots");
        assert_eq!(err.matches.len(), 2);
        assert_eq!(
            registry.names(),
            vec!["search".to_string()],
            "catalogue display names stay deduplicated even when authority roots differ"
        );
    }

    #[test]
    fn exact_authority_mode_lookup_and_remove_do_not_touch_same_name_neighbor() {
        let mut registry = AbilityControlPlaneRegistry::default();
        let owner_a = "easynet:///r/default/agent/user.a";
        let owner_b = "easynet:///r/default/agent/user.b";
        registry
            .register(
                "search",
                CallMode::Rpc,
                None,
                AuthorityScope::new("agent:a", owner_a).unwrap(),
                RuntimeEnv::new("env:a").unwrap(),
                AbilityImplSource::NativeDaemon,
            )
            .unwrap();
        registry
            .register(
                "search",
                CallMode::Rpc,
                None,
                AuthorityScope::new("agent:b", owner_b).unwrap(),
                RuntimeEnv::new("env:b").unwrap(),
                AbilityImplSource::NativeDaemon,
            )
            .unwrap();

        assert_eq!(
            registry
                .get_for_authority_mode(owner_a, "search", CallMode::Rpc)
                .unwrap()
                .implementation()
                .runtime_env()
                .label(),
            "env:a"
        );
        assert!(registry.remove_for_authority_mode(owner_a, "search", CallMode::Rpc));
        assert!(
            registry
                .get_for_authority_mode(owner_a, "search", CallMode::Rpc)
                .is_none(),
            "target authority root should be gone"
        );
        assert_eq!(
            registry
                .get_for_authority_mode(owner_b, "search", CallMode::Rpc)
                .unwrap()
                .implementation()
                .runtime_env()
                .label(),
            "env:b",
            "same public name under a different authority must survive"
        );
    }
}
