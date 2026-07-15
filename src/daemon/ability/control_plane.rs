// EasyNet CLI - Ability control-plane registry
// ============================================
//
// File: src/daemon/ability/control_plane.rs
// Description: Aggregates descriptor, authority, and implementation binding
//              registries for daemon-local ability registration.

use std::fmt;

use super::descriptors::AdmissionAction;
use super::{
    AbilityControlPlaneError, AbilityControlPlaneKey, AbilityDescriptor, AbilityDescriptorRegistry,
    AbilityHints, AbilityImplBinding, AbilityImplRegistry, AbilityImplSource, AuthorityBinding,
    AuthorityBindingRegistry, AuthorityScope, CallMode, ReceiptSemantics, RuntimeEnv,
    DEFAULT_ABILITY_DESCRIPTOR_VERSION,
};

#[derive(Debug, Clone, PartialEq)]
pub struct AbilityControlPlaneRecord {
    key: AbilityControlPlaneKey,
    descriptor: AbilityDescriptor,
    authority: AuthorityBinding,
    implementation: AbilityImplBinding,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbilityControlPlaneLookupMatch {
    pub authority_root: String,
    pub descriptor_version: String,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbilityControlPlaneAuthorityModeLookupError {
    pub authority_root: String,
    pub ability: String,
    pub call_mode: CallMode,
    pub descriptor_versions: Vec<String>,
}

impl fmt::Display for AbilityControlPlaneAuthorityModeLookupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ability {:?} under authority {:?} and call mode {:?} is ambiguous across descriptor versions {:?}",
            self.ability, self.authority_root, self.call_mode, self.descriptor_versions
        )
    }
}

impl std::error::Error for AbilityControlPlaneAuthorityModeLookupError {}

impl AbilityControlPlaneRecord {
    pub fn key(&self) -> &AbilityControlPlaneKey {
        &self.key
    }

    pub fn ability(&self) -> &str {
        self.key.ability()
    }

    pub fn descriptor(&self) -> &AbilityDescriptor {
        &self.descriptor
    }

    pub fn authority(&self) -> &AuthorityBinding {
        &self.authority
    }

    pub fn implementation(&self) -> &AbilityImplBinding {
        &self.implementation
    }
}

#[derive(Debug, Default, Clone)]
pub struct AbilityControlPlaneRegistry {
    descriptors: AbilityDescriptorRegistry,
    authorities: AuthorityBindingRegistry,
    implementations: AbilityImplRegistry,
}

#[derive(Debug, Clone)]
pub struct AbilityControlPlaneRegistration<'a> {
    ability: String,
    descriptor_version: String,
    call_mode: CallMode,
    receipt_semantics: ReceiptSemantics,
    admission_action: AdmissionAction,
    descriptor_hints: Option<AbilityHints>,
    manifest: Option<&'a crate::daemon::ability::manifest::AbilityManifest>,
    authority_scope: AuthorityScope,
    runtime_env: RuntimeEnv,
    impl_source: AbilityImplSource,
    impl_content_hash: Option<String>,
}

impl<'a> AbilityControlPlaneRegistration<'a> {
    pub fn new(
        ability: impl Into<String>,
        call_mode: CallMode,
        admission_action: AdmissionAction,
        manifest: Option<&'a crate::daemon::ability::manifest::AbilityManifest>,
        authority_scope: AuthorityScope,
        runtime_env: RuntimeEnv,
        impl_source: AbilityImplSource,
    ) -> Self {
        Self {
            ability: ability.into(),
            descriptor_version: manifest_descriptor_version(manifest).to_string(),
            call_mode,
            receipt_semantics: ReceiptSemantics::Operational,
            admission_action,
            descriptor_hints: None,
            manifest,
            authority_scope,
            runtime_env,
            impl_source,
            impl_content_hash: None,
        }
    }

    #[must_use]
    pub fn with_descriptor_version(mut self, descriptor_version: impl Into<String>) -> Self {
        self.descriptor_version = descriptor_version.into();
        self
    }

    #[must_use]
    pub fn with_impl_content_hash(mut self, impl_content_hash: impl Into<String>) -> Self {
        self.impl_content_hash = Some(impl_content_hash.into());
        self
    }

    #[must_use]
    pub fn with_receipt_semantics(mut self, receipt_semantics: ReceiptSemantics) -> Self {
        self.receipt_semantics = receipt_semantics;
        self
    }

    #[must_use]
    pub fn with_descriptor_hints(mut self, hints: AbilityHints) -> Self {
        self.descriptor_hints = Some(hints);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AbilityControlPlaneRegistrationStage {
    Planned,
    Materialized,
    Committed,
}

struct AbilityControlPlaneRegistrationPlan<'a> {
    stage: AbilityControlPlaneRegistrationStage,
    registration: AbilityControlPlaneRegistration<'a>,
}

struct MaterializedAbilityControlPlaneRegistration {
    stage: AbilityControlPlaneRegistrationStage,
    record: AbilityControlPlaneRecord,
}

impl<'a> AbilityControlPlaneRegistrationPlan<'a> {
    fn new(registration: AbilityControlPlaneRegistration<'a>) -> Self {
        Self {
            stage: AbilityControlPlaneRegistrationStage::Planned,
            registration,
        }
    }

    fn materialize(
        self,
    ) -> Result<MaterializedAbilityControlPlaneRegistration, AbilityControlPlaneError> {
        debug_assert_eq!(self.stage, AbilityControlPlaneRegistrationStage::Planned);
        let AbilityControlPlaneRegistration {
            ability,
            descriptor_version,
            call_mode,
            receipt_semantics,
            admission_action,
            descriptor_hints,
            manifest,
            authority_scope,
            runtime_env,
            impl_source,
            impl_content_hash,
        } = self.registration;
        let authority_root = authority_scope.authority_root().to_string();
        // Registry keys are implementation-qualified (`device.fs.read`,
        // `alice.chat`); Ability URAs are owner-local protocol identities.
        // Persist the canonical public projection in the descriptor record
        // while keeping the registry key unchanged for execution lookup.
        let public_ability_name =
            crate::core::ura::descriptor_public_ability_name(&authority_root, &ability);
        crate::core::ura::owner_ability_ura(&authority_root, &public_ability_name).ok_or_else(
            || AbilityControlPlaneError::DescriptorAbilityUraDerivationFailed {
                authority_root: authority_root.clone(),
                ability: public_ability_name.clone(),
            },
        )?;
        ensure_manifest_descriptor_version_matches(&descriptor_version, manifest)?;
        let mut descriptor = AbilityDescriptor::from_registry_manifest(
            &ability,
            &authority_root,
            call_mode,
            admission_action,
            manifest,
        )
        .map_err(|error| AbilityControlPlaneError::DescriptorConstruction {
            reason: error.to_string(),
        })?;
        if manifest.is_none() && descriptor.version != descriptor_version {
            descriptor = descriptor
                .with_version(&descriptor_version)
                .map_err(|error| AbilityControlPlaneError::DescriptorConstruction {
                    reason: error.to_string(),
                })?;
        }
        if let Some(hints) = descriptor_hints {
            descriptor = descriptor.with_hints(hints).with_call_mode(call_mode);
        }
        descriptor = descriptor
            .with_receipt_semantics(receipt_semantics)
            .with_source("daemon:control-plane");
        let authority = AuthorityBinding::local_self_for_descriptor(
            ability.clone(),
            authority_scope,
            &descriptor,
        )?;
        let implementation = AbilityImplBinding::new_with_content_hash(
            ability.clone(),
            descriptor.version.clone(),
            call_mode,
            runtime_env,
            impl_source,
            impl_content_hash,
        )?;
        let key = AbilityControlPlaneKey::new(
            &authority_root,
            ability,
            descriptor.version.clone(),
            descriptor.call_mode(),
        )?;
        assert_record_keys_match(&key, &descriptor, &authority, &implementation)?;
        Ok(MaterializedAbilityControlPlaneRegistration {
            stage: AbilityControlPlaneRegistrationStage::Materialized,
            record: AbilityControlPlaneRecord {
                key,
                descriptor,
                authority,
                implementation,
            },
        })
    }
}

impl MaterializedAbilityControlPlaneRegistration {
    fn commit(mut self, registry: &mut AbilityControlPlaneRegistry) -> AbilityControlPlaneRecord {
        debug_assert_eq!(
            self.stage,
            AbilityControlPlaneRegistrationStage::Materialized
        );
        registry
            .descriptors
            .register(self.record.key.clone(), self.record.descriptor.clone());
        registry.authorities.bind(self.record.authority.clone());
        registry
            .implementations
            .bind(self.record.key.clone(), self.record.implementation.clone());
        self.stage = AbilityControlPlaneRegistrationStage::Committed;
        self.record
    }
}

impl AbilityControlPlaneRegistry {
    pub fn register(
        &mut self,
        ability: impl Into<String>,
        call_mode: CallMode,
        admission_action: AdmissionAction,
        manifest: Option<&crate::daemon::ability::manifest::AbilityManifest>,
        authority_scope: AuthorityScope,
        runtime_env: RuntimeEnv,
        impl_source: AbilityImplSource,
    ) -> Result<AbilityControlPlaneRecord, AbilityControlPlaneError> {
        self.register_registration(AbilityControlPlaneRegistration::new(
            ability,
            call_mode,
            admission_action,
            manifest,
            authority_scope,
            runtime_env,
            impl_source,
        ))
    }

    pub fn register_registration(
        &mut self,
        registration: AbilityControlPlaneRegistration<'_>,
    ) -> Result<AbilityControlPlaneRecord, AbilityControlPlaneError> {
        Ok(AbilityControlPlaneRegistrationPlan::new(registration)
            .materialize()?
            .commit(self))
    }

    /// Remove every descriptor-version/call-mode row for one authority-owned
    /// ability.
    ///
    /// What this is NOT: a bare-name delete. Two authority roots may legally
    /// advertise the same public ability name, so mutation must always name the
    /// owner root that established the record.
    /// Remove every control-plane row for `ability` across all authority
    /// roots and call modes. Used where the caller knows only the ability
    /// name (e.g. test fixtures that simulate an ability whose ownership can
    /// no longer be resolved). Returns `true` if anything was removed.
    pub fn remove_for_ability(&mut self, ability: &str) -> bool {
        let descriptors_removed = self
            .descriptors
            .remove_matching(|key| key.ability() == ability);
        let authorities_removed = self
            .authorities
            .remove_matching(|key| key.ability() == ability);
        let implementations_removed = self
            .implementations
            .remove_matching(|key| key.ability() == ability);
        descriptors_removed || authorities_removed || implementations_removed
    }

    pub fn remove_for_authority(&mut self, authority_root: &str, ability: &str) -> bool {
        let descriptors_removed = self.descriptors.remove_matching(|key| {
            key.authority_root() == authority_root && key.ability() == ability
        });
        let authorities_removed = self.authorities.remove_matching(|key| {
            key.authority_root() == authority_root && key.ability() == ability
        });
        let implementations_removed = self.implementations.remove_matching(|key| {
            key.authority_root() == authority_root && key.ability() == ability
        });
        descriptors_removed || authorities_removed || implementations_removed
    }

    /// Remove every descriptor-version row for one authority-owned call mode.
    ///
    /// Use this for registration rollback where only the just-written mode is
    /// known to be part of the failed transaction. The descriptor version is
    /// deliberately not part of this key because dynamic registration and
    /// device-ability uninstall operate from runtime mode state, where the
    /// manifest-supplied version has already been folded into the record.
    /// Full ability unregister uses [`Self::remove_for_authority`] so all
    /// modes leave together.
    pub fn remove_for_authority_mode(
        &mut self,
        authority_root: &str,
        ability: &str,
        call_mode: CallMode,
    ) -> bool {
        let descriptors_removed = self.descriptors.remove_matching(|key| {
            key.authority_root() == authority_root
                && key.ability() == ability
                && key.call_mode() == call_mode
        });
        let authorities_removed = self.authorities.remove_matching(|key| {
            key.authority_root() == authority_root
                && key.ability() == ability
                && key.call_mode() == call_mode
        });
        let implementations_removed = self.implementations.remove_matching(|key| {
            key.authority_root() == authority_root
                && key.ability() == ability
                && key.call_mode() == call_mode
        });
        descriptors_removed || authorities_removed || implementations_removed
    }

    /// Snapshot every descriptor-version row for one authority-owned call mode.
    ///
    /// What this is NOT: a lookup helper for dispatch. Dispatch needs a unique
    /// descriptor version and should call [`Self::get_for_authority_mode`].
    /// Registration transactions use this method before an overwrite so rollback
    /// can restore the exact prior control-plane facts instead of merely deleting
    /// the failed write.
    pub fn records_for_authority_mode(
        &self,
        authority_root: &str,
        ability: &str,
        call_mode: CallMode,
    ) -> Vec<AbilityControlPlaneRecord> {
        self.descriptors
            .records_for_authority_mode(authority_root, ability, call_mode)
            .into_iter()
            .filter_map(|(key, _descriptor)| self.record_for_key(&key))
            .collect()
    }

    /// Replace one authority-owned call-mode slice with a prior snapshot.
    ///
    /// Invariant 1: rows outside `(authority_root, ability, call_mode)` are left
    /// untouched, so a failed Stream hot-register cannot erase an existing RPC or
    /// Bidi record.
    ///
    /// Invariant 2: records are reinserted through their canonical key derived
    /// from the record body, keeping key construction centralized in
    /// the aggregate's stored [`AbilityControlPlaneKey`].
    pub fn restore_authority_mode_records(
        &mut self,
        authority_root: &str,
        ability: &str,
        call_mode: CallMode,
        records: Vec<AbilityControlPlaneRecord>,
    ) -> Result<(), AbilityControlPlaneError> {
        self.remove_for_authority_mode(authority_root, ability, call_mode);
        for record in records {
            self.insert_materialized_record(record)?;
        }
        Ok(())
    }

    pub fn get(
        &self,
        ability: &str,
    ) -> Result<Option<AbilityControlPlaneRecord>, AbilityControlPlaneLookupError> {
        let key = unique_key_for_control_plane_default(self.descriptors.keys(), ability, None)?;
        Ok(key.and_then(|key| self.record_for_key(&key)))
    }

    pub fn get_for_mode(
        &self,
        ability: &str,
        call_mode: CallMode,
    ) -> Result<Option<AbilityControlPlaneRecord>, AbilityControlPlaneLookupError> {
        let key = unique_key_for_control_plane_default(
            self.descriptors.keys(),
            ability,
            Some(call_mode),
        )?;
        Ok(key.and_then(|key| self.record_for_key(&key)))
    }

    pub fn get_for_authority_mode(
        &self,
        authority_root: &str,
        ability: &str,
        call_mode: CallMode,
    ) -> Result<Option<AbilityControlPlaneRecord>, AbilityControlPlaneAuthorityModeLookupError>
    {
        let key = unique_key_for_authority_mode(
            self.descriptors.keys(),
            authority_root,
            ability,
            call_mode,
        )?;
        Ok(key.and_then(|key| self.record_for_key(&key)))
    }

    pub fn get_version(
        &self,
        ability: &str,
        descriptor_version: &str,
    ) -> Result<Option<AbilityControlPlaneRecord>, AbilityControlPlaneLookupError> {
        let key = unique_key_for_control_plane(
            self.descriptors.keys(),
            ability,
            descriptor_version,
            None,
        )?;
        Ok(key.and_then(|key| self.record_for_key(&key)))
    }

    pub fn get_version_for_mode(
        &self,
        ability: &str,
        descriptor_version: &str,
        call_mode: CallMode,
    ) -> Result<Option<AbilityControlPlaneRecord>, AbilityControlPlaneLookupError> {
        let key = unique_key_for_control_plane(
            self.descriptors.keys(),
            ability,
            descriptor_version,
            Some(call_mode),
        )?;
        Ok(key.and_then(|key| self.record_for_key(&key)))
    }

    pub fn get_version_for_authority_mode(
        &self,
        authority_root: &str,
        ability: &str,
        descriptor_version: &str,
        call_mode: CallMode,
    ) -> Option<AbilityControlPlaneRecord> {
        let key =
            AbilityControlPlaneKey::new(authority_root, ability, descriptor_version, call_mode)
                .ok()?;
        self.record_for_key(&key)
    }

    pub fn contains(&self, ability: &str) -> bool {
        self.descriptors
            .contains_matching(|key| key.ability() == ability)
    }

    pub fn contains_for_authority(&self, authority_root: &str, ability: &str) -> bool {
        self.descriptors.contains_matching(|key| {
            key.authority_root() == authority_root && key.ability() == ability
        })
    }

    pub fn names(&self) -> Vec<String> {
        self.descriptors.names()
    }

    /// Distinct authority roots that own `ability` across all registered modes.
    ///
    /// Runtime dispatch keys are descriptor-owner URAs, not bare ability names.
    /// Callers that need to register or invoke through `LocalRuntime` use this
    /// to prove that a name maps to one owner before deriving a protocol key.
    pub fn authority_roots_for_ability(&self, ability: &str) -> Vec<String> {
        let mut roots = self
            .descriptors
            .keys()
            .filter(|key| key.ability() == ability)
            .map(|key| key.authority_root().to_string())
            .collect::<Vec<_>>();
        roots.sort();
        roots.dedup();
        roots
    }

    /// Every registered control-plane record, joined from the descriptor,
    /// authority, and implementation facets through the shared
    /// `AbilityControlPlaneKey`. This is the single read API that
    /// `meta.list_abilities`, catalog projection, and the route resolver
    /// migrate onto so owner / manifest / authority / call-mode facts all
    /// come from one row rather than parallel side tables (SPEC §9.1.A
    /// target items 2 and 6).
    ///
    /// A key whose facets are not all present is registry corruption. Reads
    /// fail closed instead of silently dropping the governed ability.
    pub fn records(&self) -> Vec<AbilityControlPlaneRecord> {
        self.descriptors
            .keys()
            .map(|key| {
                self.record_for_key(key).unwrap_or_else(|| {
                    panic!(
                        "control-plane aggregate is missing a facet for {}",
                        format_control_plane_key(key)
                    )
                })
            })
            .collect()
    }

    fn record_for_key(&self, key: &AbilityControlPlaneKey) -> Option<AbilityControlPlaneRecord> {
        Some(AbilityControlPlaneRecord {
            key: key.clone(),
            descriptor: self.descriptors.get(key)?.clone(),
            authority: self.authorities.get(key)?.clone(),
            implementation: self.implementations.get(key)?.clone(),
        })
    }

    fn insert_materialized_record(
        &mut self,
        record: AbilityControlPlaneRecord,
    ) -> Result<(), AbilityControlPlaneError> {
        let key = record.key.clone();
        assert_record_keys_match(
            &key,
            record.descriptor(),
            record.authority(),
            record.implementation(),
        )?;
        self.descriptors.register(key.clone(), record.descriptor);
        self.authorities.bind(record.authority);
        self.implementations.bind(key, record.implementation);
        Ok(())
    }
}

fn unique_key_for_control_plane_default<'a>(
    keys: impl Iterator<Item = &'a AbilityControlPlaneKey>,
    ability: &str,
    call_mode: Option<CallMode>,
) -> Result<Option<AbilityControlPlaneKey>, AbilityControlPlaneLookupError> {
    let matches = keys
        .filter(|key| {
            key.ability() == ability && call_mode.is_none_or(|mode| key.call_mode() == mode)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Ok(None),
        [key] => Ok(Some((*key).clone())),
        _ => Err(AbilityControlPlaneLookupError {
            ability: ability.to_string(),
            descriptor_version: "<unique>".to_string(),
            matches: matches
                .iter()
                .map(|key| lookup_match_for_key(key))
                .collect(),
        }),
    }
}

fn unique_key_for_control_plane<'a>(
    keys: impl Iterator<Item = &'a AbilityControlPlaneKey>,
    ability: &str,
    descriptor_version: &str,
    call_mode: Option<CallMode>,
) -> Result<Option<AbilityControlPlaneKey>, AbilityControlPlaneLookupError> {
    let matches = keys
        .filter(|key| {
            key.ability() == ability
                && key.descriptor_version_str() == descriptor_version
                && call_mode.is_none_or(|mode| key.call_mode() == mode)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Ok(None),
        [key] => Ok(Some((*key).clone())),
        _ => Err(AbilityControlPlaneLookupError {
            ability: ability.to_string(),
            descriptor_version: descriptor_version.to_string(),
            matches: matches
                .iter()
                .map(|key| lookup_match_for_key(key))
                .collect(),
        }),
    }
}

fn unique_key_for_authority_mode<'a>(
    keys: impl Iterator<Item = &'a AbilityControlPlaneKey>,
    authority_root: &str,
    ability: &str,
    call_mode: CallMode,
) -> Result<Option<AbilityControlPlaneKey>, AbilityControlPlaneAuthorityModeLookupError> {
    let matches = keys
        .filter(|key| {
            key.authority_root() == authority_root
                && key.ability() == ability
                && key.call_mode() == call_mode
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Ok(None),
        [key] => Ok(Some((*key).clone())),
        _ => Err(AbilityControlPlaneAuthorityModeLookupError {
            authority_root: authority_root.to_string(),
            ability: ability.to_string(),
            call_mode,
            descriptor_versions: matches
                .iter()
                .map(|key| key.descriptor_version_str().to_string())
                .collect(),
        }),
    }
}

fn assert_record_keys_match(
    expected: &AbilityControlPlaneKey,
    descriptor: &AbilityDescriptor,
    authority: &AuthorityBinding,
    implementation: &AbilityImplBinding,
) -> Result<(), AbilityControlPlaneError> {
    let descriptor_key = AbilityControlPlaneKey::new(
        descriptor.owner_ura.clone(),
        expected.ability(),
        descriptor.version.clone(),
        descriptor.call_mode(),
    )?;
    assert_table_key_matches("descriptor", expected, &descriptor_key)?;
    let authority_key = authority.key();
    assert_table_key_matches("authority", expected, &authority_key)?;
    let implementation_key = implementation.key(expected.authority_root());
    assert_table_key_matches("implementation", expected, &implementation_key)?;
    Ok(())
}

fn assert_table_key_matches(
    table: &'static str,
    expected: &AbilityControlPlaneKey,
    actual: &AbilityControlPlaneKey,
) -> Result<(), AbilityControlPlaneError> {
    if expected == actual {
        return Ok(());
    }
    Err(AbilityControlPlaneError::ControlPlaneKeyMismatch {
        table,
        expected: format_control_plane_key(expected),
        actual: format_control_plane_key(actual),
    })
}

fn lookup_match_for_key(key: &AbilityControlPlaneKey) -> AbilityControlPlaneLookupMatch {
    AbilityControlPlaneLookupMatch {
        authority_root: key.authority_root().to_string(),
        descriptor_version: key.descriptor_version_str().to_string(),
        call_mode: key.call_mode(),
    }
}

fn format_control_plane_key(key: &AbilityControlPlaneKey) -> String {
    format!(
        "{}::{}@{}::{:?}",
        key.authority_root(),
        key.ability(),
        key.descriptor_version_str(),
        key.call_mode()
    )
}

fn manifest_descriptor_version(
    manifest: Option<&crate::daemon::ability::manifest::AbilityManifest>,
) -> &str {
    manifest
        .map(crate::daemon::ability::manifest::AbilityManifest::descriptor_version)
        .unwrap_or(DEFAULT_ABILITY_DESCRIPTOR_VERSION)
}

fn ensure_manifest_descriptor_version_matches(
    registration_version: &str,
    manifest: Option<&crate::daemon::ability::manifest::AbilityManifest>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const LOCAL_DEVICE_URA: &str = "easynet:///r/default/device/local";
    const LOCAL_AGENT_URA: &str = "easynet:///r/default/agent/user.assistant";

    #[test]
    fn register_writes_descriptor_authority_and_impl() {
        let mut registry = AbilityControlPlaneRegistry::default();
        let record = registry
            .register(
                "fs.read",
                CallMode::Rpc,
                AdmissionAction::Read,
                None,
                AuthorityScope::new("device", LOCAL_DEVICE_URA).unwrap(),
                RuntimeEnv::daemon_native(),
                AbilityImplSource::NativeDaemon,
            )
            .unwrap();
        assert_eq!(record.descriptor().version.as_str(), "1.0.0");
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
    fn register_uses_manifest_descriptor_version_as_control_plane_fact() {
        let manifest = crate::daemon::ability::manifest::AbilityManifest::new(
            "search",
            "search local docs",
            json!({"type": "object"}),
        )
        .unwrap()
        .with_descriptor_version("2.3.4")
        .unwrap();
        let mut registry = AbilityControlPlaneRegistry::default();
        let record = registry
            .register(
                "agent.search",
                CallMode::Rpc,
                AdmissionAction::Invoke,
                Some(&manifest),
                AuthorityScope::new("agent:assistant", LOCAL_AGENT_URA).unwrap(),
                RuntimeEnv::new("env:manifest").unwrap(),
                AbilityImplSource::NativeDaemon,
            )
            .unwrap();

        assert_eq!(record.descriptor().version.as_str(), "2.3.4");
        assert_eq!(record.authority().descriptor_version(), "2.3.4");
        assert_eq!(record.implementation().descriptor_version(), "2.3.4");
        assert!(registry
            .get_version_for_mode("agent.search", "2.3.4", CallMode::Rpc)
            .unwrap()
            .is_some());
        let default_lookup = registry
            .get_for_mode("agent.search", CallMode::Rpc)
            .expect("single non-default descriptor version is selected");
        assert_eq!(
            default_lookup.unwrap().descriptor().version.as_str(),
            "2.3.4"
        );
    }

    #[test]
    fn registration_commits_one_governed_descriptor_and_authority_policy() {
        let manifest = crate::daemon::ability::manifest::AbilityManifest::new(
            "publish",
            "publish a canonical page revision",
            json!({"type": "object"}),
        )
        .unwrap()
        .with_access(crate::daemon::ability::manifest::AccessPolicy {
            visibility: crate::daemon::ability::manifest::ManifestAccessScope::Device,
            allow_callers: Some(vec!["editor".to_string()]),
            deny_callers: Some(vec!["blocked".to_string()]),
        })
        .unwrap();
        let semantics = ReceiptSemantics::state_transition(
            "pages.publish@v1",
            crate::daemon::ability::descriptors::TransitionClass::Canonical,
        )
        .unwrap();
        let mut registry = AbilityControlPlaneRegistry::default();
        let record = registry
            .register_registration(
                AbilityControlPlaneRegistration::new(
                    "pages.publish",
                    CallMode::Rpc,
                    AdmissionAction::Invoke,
                    Some(&manifest),
                    AuthorityScope::new("agent:pages", LOCAL_AGENT_URA).unwrap(),
                    RuntimeEnv::daemon_native(),
                    AbilityImplSource::NativeDaemon,
                )
                .with_receipt_semantics(semantics.clone()),
            )
            .unwrap();

        assert_eq!(record.descriptor().receipt_semantics(), &semantics);
        assert_eq!(record.descriptor().denied_agents(), &["blocked"]);
        assert_eq!(
            record.authority().invoke_policy_hash(),
            record.descriptor().access_policy_hash_bytes(),
            "authority must bind the policy already normalized into the descriptor"
        );
    }

    #[test]
    fn register_persists_owner_local_ability_ura_without_execution_prefix() {
        let mut registry = AbilityControlPlaneRegistry::default();
        let record = registry
            .register(
                "assistant.chat",
                CallMode::Rpc,
                AdmissionAction::Invoke,
                None,
                AuthorityScope::new("agent:assistant", LOCAL_AGENT_URA).unwrap(),
                RuntimeEnv::daemon_native(),
                AbilityImplSource::NativeDaemon,
            )
            .unwrap();

        assert_eq!(record.ability(), "assistant.chat");
        assert_eq!(record.descriptor().name, "chat");
        assert_eq!(
            record.descriptor().canonical_ability_ura().as_deref(),
            Some("easynet:///r/default/ability/user.assistant.chat")
        );
        assert!(registry
            .get("assistant.chat")
            .expect("single record lookup is unambiguous")
            .is_some());
    }

    #[test]
    fn register_version_rejects_manifest_descriptor_version_mismatch() {
        let manifest = crate::daemon::ability::manifest::AbilityManifest::new(
            "search",
            "search local docs",
            json!({"type": "object"}),
        )
        .unwrap()
        .with_descriptor_version("2.3.4")
        .unwrap();
        let mut registry = AbilityControlPlaneRegistry::default();
        let err = registry
            .register_registration(
                AbilityControlPlaneRegistration::new(
                    "agent.search",
                    CallMode::Rpc,
                    AdmissionAction::Invoke,
                    Some(&manifest),
                    AuthorityScope::new("agent:assistant", LOCAL_AGENT_URA).unwrap(),
                    RuntimeEnv::new("env:manifest").unwrap(),
                    AbilityImplSource::NativeDaemon,
                )
                .with_descriptor_version("1.0.0"),
            )
            .unwrap_err();

        assert_eq!(
            err,
            AbilityControlPlaneError::DescriptorVersionMismatch {
                manifest_version: "2.3.4".to_string(),
                registration_version: "1.0.0".to_string(),
            }
        );
    }

    #[test]
    fn register_version_keeps_same_ability_versions_distinct() {
        let mut registry = AbilityControlPlaneRegistry::default();
        registry
            .register_registration(
                AbilityControlPlaneRegistration::new(
                    "fs.read",
                    CallMode::Rpc,
                    AdmissionAction::Read,
                    None,
                    AuthorityScope::new("device", LOCAL_DEVICE_URA).unwrap(),
                    RuntimeEnv::new("env:v1").unwrap(),
                    AbilityImplSource::NativeDaemon,
                )
                .with_descriptor_version("1.0.0"),
            )
            .unwrap();
        registry
            .register_registration(
                AbilityControlPlaneRegistration::new(
                    "fs.read",
                    CallMode::Rpc,
                    AdmissionAction::Read,
                    None,
                    AuthorityScope::new("device", LOCAL_DEVICE_URA).unwrap(),
                    RuntimeEnv::new("env:v2").unwrap(),
                    AbilityImplSource::NativeDaemon,
                )
                .with_descriptor_version("2.0.0"),
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
        let err = registry
            .get_for_mode("fs.read", CallMode::Rpc)
            .expect_err("mode lookup must not choose between descriptor versions");
        assert_eq!(err.descriptor_version, "<unique>");
        assert_eq!(err.matches.len(), 2);
        assert_eq!(
            err.matches
                .iter()
                .map(|m| m.descriptor_version.as_str())
                .collect::<Vec<_>>(),
            vec!["1.0.0", "2.0.0"]
        );
    }

    #[test]
    fn register_keeps_same_ability_version_modes_distinct() {
        let mut registry = AbilityControlPlaneRegistry::default();
        registry
            .register(
                "agent.chat",
                CallMode::Rpc,
                AdmissionAction::Invoke,
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
                AdmissionAction::Stream,
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

    /// SPEC §9.1.A Step 1: the row enumerators (`keys`/`records`) surface
    /// every registered `(authority, ability, call_mode)` row with its
    /// facets correctly joined, so catalog/discovery readers can consume
    /// control-plane truth instead of unioning the legacy handler maps.
    /// Same-name multi-mode abilities must appear as distinct rows.
    #[test]
    fn keys_and_records_enumerate_every_authority_mode_row() {
        let mut registry = AbilityControlPlaneRegistry::default();
        registry
            .register(
                "agent.chat",
                CallMode::Rpc,
                AdmissionAction::Invoke,
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
                AdmissionAction::Stream,
                None,
                AuthorityScope::new("agent:assistant", LOCAL_AGENT_URA).unwrap(),
                RuntimeEnv::new("env:stream").unwrap(),
                AbilityImplSource::NativeDaemon,
            )
            .unwrap();

        let records = registry.records();
        assert_eq!(records.len(), 2, "both call-mode rows must enumerate");

        // Each enumerated record's key must match its joined facets — proving
        // the rows are the same row the typed-key lookups return, not a union
        // artifact.
        for record in &records {
            assert_eq!(record.key().ability(), "agent.chat");
            assert_eq!(record.key().call_mode(), record.descriptor().call_mode());
            assert_eq!(
                record.key().authority_root(),
                record.authority().scope().authority_root()
            );
        }

        let mut modes: Vec<CallMode> = records
            .iter()
            .map(|record| record.key().call_mode())
            .collect();
        modes.sort();
        assert_eq!(modes, vec![CallMode::Rpc, CallMode::Stream]);
    }

    #[test]
    fn register_keeps_same_public_name_distinct_across_authority_roots() {
        let mut registry = AbilityControlPlaneRegistry::default();
        registry
            .register(
                "search",
                CallMode::Rpc,
                AdmissionAction::Invoke,
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
                AdmissionAction::Invoke,
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
                AdmissionAction::Invoke,
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
                AdmissionAction::Invoke,
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
                .unwrap()
                .is_none(),
            "target authority root should be gone"
        );
        assert_eq!(
            registry
                .get_for_authority_mode(owner_b, "search", CallMode::Rpc)
                .unwrap()
                .unwrap()
                .implementation()
                .runtime_env()
                .label(),
            "env:b",
            "same public name under a different authority must survive"
        );
    }

    #[test]
    fn exact_authority_remove_clears_all_modes_without_touching_neighbor() {
        let mut registry = AbilityControlPlaneRegistry::default();
        let owner_a = "easynet:///r/default/agent/user.a";
        let owner_b = "easynet:///r/default/agent/user.b";
        registry
            .register(
                "search",
                CallMode::Rpc,
                AdmissionAction::Invoke,
                None,
                AuthorityScope::new("agent:a", owner_a).unwrap(),
                RuntimeEnv::new("env:a-rpc").unwrap(),
                AbilityImplSource::NativeDaemon,
            )
            .unwrap();
        registry
            .register(
                "search",
                CallMode::Stream,
                AdmissionAction::Stream,
                None,
                AuthorityScope::new("agent:a", owner_a).unwrap(),
                RuntimeEnv::new("env:a-stream").unwrap(),
                AbilityImplSource::NativeDaemon,
            )
            .unwrap();
        registry
            .register(
                "search",
                CallMode::Rpc,
                AdmissionAction::Invoke,
                None,
                AuthorityScope::new("agent:b", owner_b).unwrap(),
                RuntimeEnv::new("env:b-rpc").unwrap(),
                AbilityImplSource::NativeDaemon,
            )
            .unwrap();

        assert!(registry.remove_for_authority(owner_a, "search"));

        assert!(!registry.contains_for_authority(owner_a, "search"));
        assert!(
            registry.contains_for_authority(owner_b, "search"),
            "authority-scoped removal must not delete another owner root"
        );
        assert_eq!(
            registry
                .get_for_authority_mode(owner_b, "search", CallMode::Rpc)
                .unwrap()
                .unwrap()
                .implementation()
                .runtime_env()
                .label(),
            "env:b-rpc"
        );
    }

    #[test]
    fn authority_mode_lookup_and_remove_are_not_default_version_bound() {
        let manifest = crate::daemon::ability::manifest::AbilityManifest::new(
            "search",
            "search local docs",
            json!({"type": "object"}),
        )
        .unwrap()
        .with_descriptor_version("2.0.0")
        .unwrap();
        let mut registry = AbilityControlPlaneRegistry::default();
        registry
            .register(
                "search",
                CallMode::Rpc,
                AdmissionAction::Invoke,
                Some(&manifest),
                AuthorityScope::new("agent:a", LOCAL_AGENT_URA).unwrap(),
                RuntimeEnv::new("env:v2").unwrap(),
                AbilityImplSource::NativeDaemon,
            )
            .unwrap();

        assert_eq!(
            registry
                .get_for_authority_mode(LOCAL_AGENT_URA, "search", CallMode::Rpc)
                .unwrap()
                .unwrap()
                .descriptor()
                .version
                .as_str(),
            "2.0.0"
        );
        assert!(registry.remove_for_authority_mode(LOCAL_AGENT_URA, "search", CallMode::Rpc));
        assert!(registry
            .get_for_authority_mode(LOCAL_AGENT_URA, "search", CallMode::Rpc)
            .unwrap()
            .is_none());
    }
}
