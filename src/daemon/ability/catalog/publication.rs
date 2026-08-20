//! Immutable publication view captured from the live ability control plane.
//!
//! Route admission and directory publication consume the same committed
//! `AxonAbilityCatalog` records. This prevents resolver listings from drifting
//! away from hot registrations that are already executable by `LocalRuntime`.

use std::collections::{BTreeMap, HashSet};

use serde_json::Value;

use crate::daemon::ability::descriptors::AbilityDescriptor;
use crate::daemon::ability::dispatch::{AxonAbilityCatalog, CatalogRuntimeBindingState, OwnerKind};
use crate::daemon::ability::{AbilityImplBinding, AuthorityBinding};

#[derive(Debug, Clone, Default)]
pub(crate) struct LocalAbilityPublicationSnapshot {
    publications_by_owner: BTreeMap<String, Vec<AbilityPublication>>,
    ability_uras: HashSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalPublishedAbility {
    pub public_name: String,
    pub callee_ura: String,
    pub execution_host_ura: String,
    pub descriptor_ref: String,
    pub dispatch_name: String,
}

/// Complete local publication read model for one committed ability row.
///
/// Descriptor contract, owner/callee binding, authority binding,
/// implementation binding, and route binding remain distinct facets. Keeping
/// them together here prevents route and directory code from rebuilding public
/// ability identity from flat registry names or treating the execution host as
/// the callee.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AbilityPublication {
    descriptor_contract: AbilityDescriptor,
    owner_binding: OwnerBinding,
    authority_binding: AuthorityBinding,
    implementation_binding: AbilityImplBinding,
    route_binding: PublicationRouteBinding,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnerBinding {
    owner: OwnerKind,
    owner_ura: String,
    ability_ura: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PublicationRouteBinding {
    callee_ura: String,
    execution_host_ura: String,
    descriptor_ref: String,
    dispatch_key: String,
}

impl AbilityPublication {
    fn from_catalog_row(
        row: crate::daemon::ability::dispatch::AuthorityAbilityCatalogSnapshotRow,
    ) -> Option<Self> {
        let descriptor = row.descriptor;
        let ability_ura = descriptor.canonical_ability_ura()?;
        let descriptor_ref = descriptor.descriptor_ref().ok()?;
        let owner_ura = descriptor.owner_ura.clone();
        if !row.owner.matches_owner_ura(&owner_ura) {
            return None;
        }
        if !row
            .owner
            .execution_host_matches_owner_ura(&owner_ura, &row.execution_host_ura)
        {
            return None;
        }
        if row.runtime_binding.state != CatalogRuntimeBindingState::Bound {
            return None;
        }
        if !authority_binding_matches_publication(
            &row.authority,
            &row.owner,
            &owner_ura,
            &descriptor,
            &row.name,
        ) {
            return None;
        }
        if !implementation_binding_matches_publication(&row.implementation, &descriptor, &row.name)
        {
            return None;
        }
        Some(Self {
            descriptor_contract: descriptor,
            owner_binding: OwnerBinding {
                owner: row.owner,
                owner_ura: owner_ura.clone(),
                ability_ura,
            },
            authority_binding: row.authority,
            implementation_binding: row.implementation,
            route_binding: PublicationRouteBinding {
                callee_ura: owner_ura,
                execution_host_ura: row.execution_host_ura,
                descriptor_ref,
                dispatch_key: row.name,
            },
        })
    }

    fn descriptor(&self) -> &AbilityDescriptor {
        &self.descriptor_contract
    }

    fn owner_ura(&self) -> &str {
        &self.owner_binding.owner_ura
    }

    fn ability_ura(&self) -> &str {
        &self.owner_binding.ability_ura
    }

    fn descriptor_ref(&self) -> &str {
        &self.route_binding.descriptor_ref
    }

    fn dispatch_key(&self) -> &str {
        &self.route_binding.dispatch_key
    }

    fn is_routable_local_publication(&self) -> bool {
        super::is_local_runtime_routable_catalog_name(self.dispatch_key())
    }
}

fn authority_binding_matches_publication(
    authority: &AuthorityBinding,
    owner: &OwnerKind,
    owner_ura: &str,
    descriptor: &AbilityDescriptor,
    dispatch_key: &str,
) -> bool {
    authority.ability() == dispatch_key
        && authority.descriptor_version() == descriptor.version
        && authority.call_mode() == descriptor.call_mode()
        && authority.scope().authority_root() == owner_ura
        && authority.scope().owner_projection() == owner.authority_projection()
}

fn implementation_binding_matches_publication(
    implementation: &AbilityImplBinding,
    descriptor: &AbilityDescriptor,
    dispatch_key: &str,
) -> bool {
    implementation.ability() == dispatch_key
        && implementation.descriptor_version() == descriptor.version
        && implementation.call_mode() == descriptor.call_mode()
}

impl LocalAbilityPublicationSnapshot {
    #[must_use]
    pub(crate) fn capture(catalog: &AxonAbilityCatalog) -> Self {
        let mut snapshot = Self::default();
        for row in catalog.authority_ability_catalog_snapshot() {
            let Some(publication) = AbilityPublication::from_catalog_row(row) else {
                continue;
            };
            if !publication.is_routable_local_publication() {
                continue;
            }
            snapshot
                .ability_uras
                .insert(publication.ability_ura().to_string());
            snapshot
                .publications_by_owner
                .entry(publication.owner_ura().to_string())
                .or_default()
                .push(publication);
        }
        snapshot
    }

    pub(crate) fn resolve_with_call_mode(
        &self,
        owner_ura: &str,
        public_name: &str,
        call_mode: crate::daemon::ability::CallMode,
    ) -> Option<LocalPublishedAbility> {
        let ability_ura = crate::core::ura::owner_ability_ura(owner_ura, public_name)?;
        if !self.ability_uras.contains(&ability_ura) {
            return None;
        }
        let publication =
            self.publications_by_owner
                .get(owner_ura)?
                .iter()
                .find(|publication| {
                    publication.descriptor().public_name() == public_name
                        && publication.descriptor().call_mode() == call_mode
                })?;
        Some(LocalPublishedAbility {
            public_name: public_name.to_string(),
            callee_ura: publication.route_binding.callee_ura.clone(),
            execution_host_ura: publication.route_binding.execution_host_ura.clone(),
            descriptor_ref: publication.descriptor_ref().to_string(),
            dispatch_name: publication.dispatch_key().to_string(),
        })
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) fn resolves(&self, owner_ura: &str, public_name: &str) -> bool {
        self.resolve_with_call_mode(
            owner_ura,
            public_name,
            crate::daemon::ability::CallMode::Rpc,
        )
        .is_some()
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) fn resolves_with_call_mode(
        &self,
        owner_ura: &str,
        public_name: &str,
        call_mode: crate::daemon::ability::CallMode,
    ) -> bool {
        self.resolve_with_call_mode(owner_ura, public_name, call_mode)
            .is_some()
    }

    pub(crate) fn owner_projection_values(&self, owner_ura: &str) -> Result<Vec<Value>, String> {
        self.publications_by_owner
            .get(owner_ura)
            .into_iter()
            .flatten()
            .map(|publication| {
                crate::daemon::federation::read_model::owner_projection::summary_from_descriptor(
                    publication.descriptor(),
                )
                .map_err(|error| {
                    format!(
                        "local ability publication for owner `{owner_ura}` descriptor `{}` is invalid: {error}",
                        publication.descriptor().name
                    )
                })
            })
            .map(|summary| {
                summary.and_then(|summary| {
                    serde_json::to_value(summary).map_err(|error| {
                        format!(
                            "local ability publication for owner `{owner_ura}` cannot serialize summary: {error}"
                        )
                    })
                })
            })
            .collect()
    }

    #[must_use]
    pub(crate) fn owner_descriptors(&self, owner_ura: &str) -> Vec<AbilityDescriptor> {
        self.publications_by_owner
            .get(owner_ura)
            .into_iter()
            .flatten()
            .map(|publication| publication.descriptor().clone())
            .collect()
    }

    /// Return committed descriptors for one locally-hosted Agent id.
    ///
    /// Agent roster metadata and ability publication have different owners:
    /// `AgentRegistry` says which runtime/model is configured, while this
    /// snapshot says which governed handlers actually committed. Consumers
    /// such as A2A join the two projections by the canonical Agent id and must
    /// never reopen manifests to reconstruct capability rows.
    #[must_use]
    pub(crate) fn hosted_agent_descriptors(&self, agent_id: &str) -> Vec<AbilityDescriptor> {
        let matching_owners = self
            .publications_by_owner
            .iter()
            .filter(|(owner_ura, _)| {
                crate::core::ura::parse_ura(owner_ura)
                    .ok()
                    .filter(|parsed| parsed.kind == crate::core::ura::URAKind::Agent)
                    .and_then(|parsed| parsed.agent_ids().map(|(_, id)| id == agent_id))
                    .unwrap_or(false)
            })
            .collect::<Vec<_>>();
        if matching_owners.len() != 1 {
            return Vec::new();
        }
        let mut descriptors = matching_owners[0]
            .1
            .iter()
            .map(|publication| publication.descriptor().clone())
            .collect::<Vec<_>>();
        descriptors.sort_by(|left, right| {
            left.public_name()
                .cmp(&right.public_name())
                .then_with(|| left.call_mode().as_str().cmp(right.call_mode().as_str()))
                .then_with(|| left.version.cmp(&right.version))
        });
        descriptors
    }

    /// Return every committed local owner that is itself a hosted Agent.
    ///
    /// This is the publication-side authority for session reconnect recovery:
    /// if the committed LocalRuntime catalog contains routable descriptors for
    /// an Agent owner, the federation publisher must re-advertise that owner
    /// identity and its complete ability projection after the upstream Hub
    /// session becomes ready. Product-specific agents such as Pages, Files, or
    /// MCP must not maintain separate recovery paths.
    #[must_use]
    pub(crate) fn hosted_agent_owner_uras(&self) -> Vec<String> {
        self.publications_by_owner
            .keys()
            .filter(|owner_ura| {
                crate::core::ura::parse_ura(owner_ura)
                    .ok()
                    .is_some_and(|parsed| {
                        parsed.kind == crate::core::ura::URAKind::Agent
                            && parsed.device_agent_ids().is_none()
                            && parsed.agent_ids().is_some()
                    })
            })
            .cloned()
            .collect()
    }

    /// Return every committed local owner that is a Device-sponsored
    /// SystemAgent.
    ///
    /// SystemAgent descriptors are independently routable owner projections;
    /// their sponsoring Device supplies execution/custody only. Dynamic
    /// plugin and deployed-ability publication must therefore enumerate these
    /// owners directly instead of collapsing them into a Device projection.
    #[must_use]
    pub(crate) fn system_agent_owner_uras(&self) -> Vec<String> {
        self.publications_by_owner
            .keys()
            .filter(|owner_ura| {
                crate::core::ura::parse_ura(owner_ura)
                    .ok()
                    .is_some_and(|parsed| {
                        parsed.kind == crate::core::ura::URAKind::Agent
                            && parsed.device_agent_ids().is_some()
                    })
            })
            .cloned()
            .collect()
    }

    /// Return every committed local owner that is a user-scoped Service.
    ///
    /// Service surfaces (Pages, Files) are independently routable owner
    /// projections executed on a hosting Device. The federation publisher
    /// must enumerate them alongside SystemAgents: a Service that is absent
    /// from the Hub owner projection cannot serve public routes (e.g.
    /// `/web/<user>/<project>/` resolving `<user>.<project>.page.fetch`)
    /// even though local dispatch works.
    #[must_use]
    pub(crate) fn service_owner_uras(&self) -> Vec<String> {
        self.publications_by_owner
            .keys()
            .filter(|owner_ura| {
                crate::core::ura::parse_ura(owner_ura)
                    .ok()
                    .is_some_and(|parsed| parsed.kind == crate::core::ura::URAKind::Service)
            })
            .cloned()
            .collect()
    }

    /// Select the unique callable SystemAgent owner that this live snapshot
    /// proves for one Device placement and public ability. This is the dynamic
    /// plugin/deployment counterpart to the deterministic system registry:
    /// routing may use committed catalog evidence, but never infer an owner
    /// from an ability prefix or plugin name.
    #[must_use]
    pub(crate) fn unique_system_agent_owner_for_device_ability(
        &self,
        device_ura: &str,
        public_name: &str,
    ) -> Option<String> {
        let device = crate::core::ura::parse_ura(device_ura).ok()?;
        let device_id = device.device_id()?;
        let public_name = public_name.trim();
        if public_name.is_empty() {
            return None;
        }

        let mut owners = self
            .publications_by_owner
            .iter()
            .filter_map(|(owner_ura, publications)| {
                let owner = crate::core::ura::parse_ura(owner_ura).ok()?;
                let (owner_device_id, _) = owner.device_agent_ids()?;
                if owner.realm != device.realm
                    || owner_device_id != device_id
                    || !publications
                        .iter()
                        .any(|publication| publication.descriptor().public_name() == public_name)
                {
                    return None;
                }
                Some(owner_ura.clone())
            })
            .collect::<Vec<_>>();
        owners.sort();
        owners.dedup();
        match owners.as_slice() {
            [owner] => Some(owner.clone()),
            _ => None,
        }
    }

    #[must_use]
    pub(crate) fn all_descriptors(&self) -> Vec<AbilityDescriptor> {
        self.publications_by_owner
            .values()
            .flatten()
            .map(|publication| publication.descriptor().clone())
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn from_owner_public_names(owner_ura: &str, public_names: &[&str]) -> Self {
        let execution_host_ura = default_test_execution_host_for_owner(owner_ura);
        Self::from_owner_public_names_on_host(owner_ura, &execution_host_ura, public_names)
    }

    #[cfg(test)]
    pub(crate) fn from_owner_public_names_on_host(
        owner_ura: &str,
        execution_host_ura: &str,
        public_names: &[&str],
    ) -> Self {
        use crate::daemon::ability::descriptors::{AdmissionAction, Visibility};

        let mut snapshot = Self::default();
        for public_name in public_names {
            let descriptor = AbilityDescriptor::new(
                (*public_name).to_string(),
                owner_ura.to_string(),
                Visibility::Scoped,
                AdmissionAction::Invoke,
            )
            .expect("test publication descriptor");
            let dispatch_key = crate::core::ura::local_dispatch_ability_key(owner_ura, public_name);
            snapshot.insert_test_descriptor(descriptor, execution_host_ura, &dispatch_key);
        }
        snapshot
    }

    #[cfg(test)]
    pub(crate) fn from_descriptors(descriptors: Vec<AbilityDescriptor>) -> Self {
        let mut snapshot = Self::default();
        for descriptor in descriptors {
            let dispatch_key = crate::core::ura::local_dispatch_ability_key(
                &descriptor.owner_ura,
                &descriptor.public_name(),
            );
            let execution_host_ura = default_test_execution_host_for_owner(&descriptor.owner_ura);
            snapshot.insert_test_descriptor(descriptor, &execution_host_ura, &dispatch_key);
        }
        snapshot
    }

    #[cfg(test)]
    fn insert_test_descriptor(
        &mut self,
        descriptor: AbilityDescriptor,
        execution_host_ura: &str,
        dispatch_key: &str,
    ) {
        let ability_ura = descriptor
            .canonical_ability_ura()
            .expect("test descriptor must derive a canonical Ability URA");
        let owner_ura = descriptor.owner_ura.clone();
        let owner = test_owner_kind_for_owner_ura(&owner_ura);
        let authority_scope = crate::daemon::ability::AuthorityScope::new(
            owner.authority_projection(),
            owner_ura.clone(),
        )
        .expect("test authority scope");
        let authority_binding =
            crate::daemon::ability::AuthorityBinding::local_self_for_descriptor(
                dispatch_key.to_string(),
                authority_scope,
                &descriptor,
            )
            .expect("test authority binding");
        let implementation_binding = crate::daemon::ability::AbilityImplBinding::new(
            dispatch_key.to_string(),
            descriptor.version.clone(),
            descriptor.call_mode(),
            crate::daemon::ability::RuntimeEnv::daemon_native(),
            crate::daemon::ability::AbilityImplSource::Test,
        )
        .expect("test implementation binding");
        let owner_matches = owner.matches_owner_ura(&owner_ura);
        let host_matches = owner.execution_host_matches_owner_ura(&owner_ura, execution_host_ura);
        let authority_matches = authority_binding_matches_publication(
            &authority_binding,
            &owner,
            &owner_ura,
            &descriptor,
            dispatch_key,
        );
        let implementation_matches = implementation_binding_matches_publication(
            &implementation_binding,
            &descriptor,
            dispatch_key,
        );
        let publication = AbilityPublication::from_catalog_row(
            crate::daemon::ability::dispatch::AuthorityAbilityCatalogSnapshotRow {
                name: dispatch_key.to_string(),
                owner,
                execution_host_ura: execution_host_ura.to_string(),
                descriptor,
                authority: authority_binding,
                implementation: implementation_binding,
                runtime_binding: crate::daemon::ability::dispatch::CatalogRuntimeBinding {
                    state: crate::daemon::ability::dispatch::CatalogRuntimeBindingState::Bound,
                    implementation_source: "test".to_string(),
                    runtime_env: "daemon-native".to_string(),
                },
            },
        )
        .unwrap_or_else(|| {
            panic!(
                "test descriptor `{dispatch_key}` for owner `{owner_ura}` on host `{execution_host_ura}` must materialize through the same publication gate: owner_matches={owner_matches} host_matches={host_matches} authority_matches={authority_matches} implementation_matches={implementation_matches}"
            )
        });
        self.ability_uras.insert(ability_ura);
        self.publications_by_owner
            .entry(owner_ura)
            .or_default()
            .push(publication);
    }
}

#[cfg(test)]
fn test_owner_kind_for_owner_ura(owner_ura: &str) -> OwnerKind {
    let parsed = crate::core::ura::parse_ura(owner_ura)
        .expect("test publication owner must be a canonical URA");
    match parsed.kind {
        crate::core::ura::URAKind::Device => OwnerKind::DeviceProfileProjection,
        crate::core::ura::URAKind::Authority => OwnerKind::RealmAuthority,
        crate::core::ura::URAKind::Service => {
            let (principal_id, service_id) = parsed
                .service_ids()
                .expect("canonical service owner must expose service ids");
            OwnerKind::Service {
                principal_id: principal_id.to_string(),
                service_id: service_id.to_string(),
            }
        }
        crate::core::ura::URAKind::Agent => {
            if let Some((_device_id, agent_id)) = parsed.device_agent_ids() {
                OwnerKind::SystemAgent(agent_id.to_string())
            } else {
                let (_user_id, agent_id) = parsed
                    .agent_ids()
                    .expect("canonical hosted Agent owner must expose agent ids");
                OwnerKind::Agent(agent_id.to_string())
            }
        }
        other => panic!("unsupported test publication owner kind: {other:?}"),
    }
}

#[cfg(test)]
fn default_test_execution_host_for_owner(owner_ura: &str) -> String {
    let Ok(parsed) = crate::core::ura::parse_ura(owner_ura) else {
        return owner_ura.to_string();
    };
    match parsed.kind {
        crate::core::ura::URAKind::Agent => {
            if let Some((device_id, _)) = parsed.device_agent_ids() {
                crate::core::ura::device_ura(&parsed.realm, device_id)
            } else {
                crate::core::ura::device_ura(&parsed.realm, "test-host")
            }
        }
        crate::core::ura::URAKind::Service => {
            crate::core::ura::device_ura(&parsed.realm, "test-host")
        }
        _ => owner_ura.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::daemon::ability::dispatch::{AbilityAuthorityContext, OwnerKind};

    #[test]
    fn resolve_selects_descriptor_by_call_mode_not_public_name_only() {
        let owner_ura =
            crate::core::ura::device_agent_ura("acme", "node-a", "runtime-introspection");
        let rpc = AbilityDescriptor::new(
            "runtime.events",
            owner_ura.clone(),
            crate::daemon::ability::descriptors::Visibility::Scoped,
            crate::daemon::ability::descriptors::AdmissionAction::Invoke,
        )
        .expect("rpc descriptor");
        let stream = AbilityDescriptor::new(
            "runtime.events",
            owner_ura.clone(),
            crate::daemon::ability::descriptors::Visibility::Scoped,
            crate::daemon::ability::descriptors::AdmissionAction::Stream,
        )
        .expect("stream descriptor")
        .with_call_mode(crate::daemon::ability::CallMode::Stream);
        let rpc_ref = rpc.descriptor_ref().expect("rpc descriptor ref");
        let stream_ref = stream.descriptor_ref().expect("stream descriptor ref");
        let snapshot = LocalAbilityPublicationSnapshot::from_descriptors(vec![rpc, stream]);

        assert_eq!(
            snapshot
                .resolve_with_call_mode(
                    &owner_ura,
                    "runtime.events",
                    crate::daemon::ability::CallMode::Rpc,
                )
                .expect("rpc publication")
                .descriptor_ref,
            rpc_ref
        );
        assert_eq!(
            snapshot
                .resolve_with_call_mode(
                    &owner_ura,
                    "runtime.events",
                    crate::daemon::ability::CallMode::Stream,
                )
                .expect("stream publication")
                .descriptor_ref,
            stream_ref
        );
        assert!(!snapshot.resolves_with_call_mode(
            &owner_ura,
            "runtime.events",
            crate::daemon::ability::CallMode::Bidi,
        ));
    }

    #[test]
    fn capture_tracks_system_agent_commits_without_mutating_prior_snapshot() {
        let device_profile_owner_ura = "easynet:///r/acme/device/node-a";
        let system_agent_owner_ura =
            crate::core::ura::device_agent_ura("acme", "node-a", "runtime-introspection");
        let catalog = AxonAbilityCatalog::new_with_runtime_and_authority_context(
            crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
                crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
                None,
            ),
            AbilityAuthorityContext::for_device_authority_root(device_profile_owner_ura)
                .expect("device authority context"),
        );
        let before = LocalAbilityPublicationSnapshot::capture(&catalog);

        catalog
            .hot_register_rpc_with_spec(
                "runtime.cursor",
                OwnerKind::runtime_introspection_system(),
                crate::daemon::ability::manifest::AbilityManifest::new(
                    "dynamic",
                    "test SystemAgent cursor ability",
                    serde_json::json!({"type": "object"}),
                )
                .and_then(|manifest| manifest.with_admission_action("invoke"))
                .expect("test manifest"),
                Arc::new(|_args| Ok(serde_json::json!({"ok": true}))),
            )
            .expect("hot-register dynamic ability");
        let after = LocalAbilityPublicationSnapshot::capture(&catalog);

        assert!(!before.resolves(&system_agent_owner_ura, "runtime.cursor"));
        assert!(after.resolves(&system_agent_owner_ura, "runtime.cursor"));
        let published = after
            .owner_projection_values(&system_agent_owner_ura)
            .expect("local publication must project");
        assert!(published.iter().any(|summary| {
            summary.get("namespace").and_then(Value::as_str) == Some("runtime")
                && summary.get("local_name").and_then(Value::as_str) == Some("cursor")
        }));
    }

    #[test]
    fn capture_materializes_complete_ability_publication_facets() {
        let device_profile_owner_ura = "easynet:///r/acme/device/node-a";
        let system_agent_owner_ura =
            crate::core::ura::device_agent_ura("acme", "node-a", "runtime-introspection");
        let catalog = AxonAbilityCatalog::new_with_runtime_and_authority_context(
            crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
                crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
                None,
            ),
            AbilityAuthorityContext::for_device_authority_root(device_profile_owner_ura)
                .expect("device authority context"),
        );
        catalog
            .hot_register_rpc_with_spec(
                "runtime.cursor",
                OwnerKind::runtime_introspection_system(),
                crate::daemon::ability::manifest::AbilityManifest::new(
                    "dynamic",
                    "test complete publication facets",
                    serde_json::json!({"type": "object"}),
                )
                .and_then(|manifest| manifest.with_admission_action("invoke"))
                .expect("test manifest"),
                Arc::new(|_args| Ok(serde_json::json!({"ok": true}))),
            )
            .expect("hot-register dynamic ability");

        let snapshot = LocalAbilityPublicationSnapshot::capture(&catalog);
        let publication = snapshot
            .publications_by_owner
            .get(&system_agent_owner_ura)
            .and_then(|publications| {
                publications
                    .iter()
                    .find(|publication| publication.descriptor().public_name() == "runtime.cursor")
            })
            .expect("complete publication");

        assert_eq!(
            publication.descriptor_contract.owner_ura,
            system_agent_owner_ura
        );
        assert_eq!(
            publication.owner_binding.owner,
            OwnerKind::runtime_introspection_system()
        );
        assert_eq!(publication.owner_binding.owner_ura, system_agent_owner_ura);
        assert_eq!(publication.route_binding.callee_ura, system_agent_owner_ura);
        assert_eq!(
            publication.route_binding.execution_host_ura,
            device_profile_owner_ura
        );
        assert_eq!(publication.route_binding.dispatch_key, "runtime.cursor");
        assert_eq!(
            publication.authority_binding.scope().authority_root(),
            system_agent_owner_ura
        );
        assert_eq!(
            publication.implementation_binding.ability(),
            "runtime.cursor"
        );
    }

    #[test]
    fn publication_row_rejects_owner_kind_ura_mismatch() {
        let device_profile_owner_ura = "easynet:///r/acme/device/node-a";
        let system_agent_owner_ura =
            crate::core::ura::device_agent_ura("acme", "node-a", "runtime-introspection");
        let descriptor = AbilityDescriptor::new(
            "runtime.cursor",
            system_agent_owner_ura.clone(),
            crate::daemon::ability::descriptors::Visibility::Scoped,
            crate::daemon::ability::descriptors::AdmissionAction::Invoke,
        )
        .expect("test descriptor");
        let authority_scope = crate::daemon::ability::AuthorityScope::new(
            "system-agent:runtime-introspection",
            system_agent_owner_ura.clone(),
        )
        .expect("test authority scope");
        let authority = crate::daemon::ability::AuthorityBinding::local_self_for_descriptor(
            "runtime.cursor",
            authority_scope,
            &descriptor,
        )
        .expect("test authority binding");
        let implementation = crate::daemon::ability::AbilityImplBinding::new(
            "runtime.cursor",
            descriptor.version.clone(),
            descriptor.call_mode(),
            crate::daemon::ability::RuntimeEnv::daemon_native(),
            crate::daemon::ability::AbilityImplSource::Test,
        )
        .expect("test implementation binding");
        let row = crate::daemon::ability::dispatch::AuthorityAbilityCatalogSnapshotRow {
            name: "runtime.cursor".to_string(),
            owner: OwnerKind::Service {
                principal_id: "alice".to_string(),
                service_id: "pages".to_string(),
            },
            execution_host_ura: device_profile_owner_ura.to_string(),
            descriptor,
            authority,
            implementation,
            runtime_binding: crate::daemon::ability::dispatch::CatalogRuntimeBinding {
                state: crate::daemon::ability::dispatch::CatalogRuntimeBindingState::Bound,
                implementation_source: "test".to_string(),
                runtime_env: "daemon-native".to_string(),
            },
        };

        assert!(
            AbilityPublication::from_catalog_row(row).is_none(),
            "publication must fail closed when OwnerKind and descriptor owner URA diverge"
        );
    }

    #[test]
    fn publication_row_rejects_authority_binding_scope_mismatch() {
        let device_profile_owner_ura = "easynet:///r/acme/device/node-a";
        let system_agent_owner_ura =
            crate::core::ura::device_agent_ura("acme", "node-a", "runtime-introspection");
        let descriptor = AbilityDescriptor::new(
            "runtime.cursor",
            system_agent_owner_ura.clone(),
            crate::daemon::ability::descriptors::Visibility::Scoped,
            crate::daemon::ability::descriptors::AdmissionAction::Invoke,
        )
        .expect("test descriptor");
        let authority_scope =
            crate::daemon::ability::AuthorityScope::new("device", device_profile_owner_ura)
                .expect("test authority scope");
        let authority = crate::daemon::ability::AuthorityBinding::local_self_for_descriptor(
            "runtime.cursor",
            authority_scope,
            &descriptor,
        )
        .expect("test authority binding");
        let implementation = crate::daemon::ability::AbilityImplBinding::new(
            "runtime.cursor",
            descriptor.version.clone(),
            descriptor.call_mode(),
            crate::daemon::ability::RuntimeEnv::daemon_native(),
            crate::daemon::ability::AbilityImplSource::Test,
        )
        .expect("test implementation binding");
        let row = crate::daemon::ability::dispatch::AuthorityAbilityCatalogSnapshotRow {
            name: "runtime.cursor".to_string(),
            owner: OwnerKind::runtime_introspection_system(),
            execution_host_ura: device_profile_owner_ura.to_string(),
            descriptor,
            authority,
            implementation,
            runtime_binding: crate::daemon::ability::dispatch::CatalogRuntimeBinding {
                state: crate::daemon::ability::dispatch::CatalogRuntimeBindingState::Bound,
                implementation_source: "test".to_string(),
                runtime_env: "daemon-native".to_string(),
            },
        };

        assert!(
            AbilityPublication::from_catalog_row(row).is_none(),
            "publication must fail closed when AuthorityBinding scope is not the callable owner"
        );
    }

    #[test]
    fn publication_row_rejects_implementation_binding_mismatch() {
        let device_profile_owner_ura = "easynet:///r/acme/device/node-a";
        let system_agent_owner_ura =
            crate::core::ura::device_agent_ura("acme", "node-a", "runtime-introspection");
        let descriptor = AbilityDescriptor::new(
            "runtime.cursor",
            system_agent_owner_ura.clone(),
            crate::daemon::ability::descriptors::Visibility::Scoped,
            crate::daemon::ability::descriptors::AdmissionAction::Invoke,
        )
        .expect("test descriptor");
        let authority_scope = crate::daemon::ability::AuthorityScope::new(
            "system-agent:runtime-introspection",
            system_agent_owner_ura.clone(),
        )
        .expect("test authority scope");
        let authority = crate::daemon::ability::AuthorityBinding::local_self_for_descriptor(
            "runtime.cursor",
            authority_scope,
            &descriptor,
        )
        .expect("test authority binding");
        let implementation = crate::daemon::ability::AbilityImplBinding::new(
            "runtime.cursor.stale",
            descriptor.version.clone(),
            descriptor.call_mode(),
            crate::daemon::ability::RuntimeEnv::daemon_native(),
            crate::daemon::ability::AbilityImplSource::Test,
        )
        .expect("test implementation binding");
        let row = crate::daemon::ability::dispatch::AuthorityAbilityCatalogSnapshotRow {
            name: "runtime.cursor".to_string(),
            owner: OwnerKind::runtime_introspection_system(),
            execution_host_ura: device_profile_owner_ura.to_string(),
            descriptor,
            authority,
            implementation,
            runtime_binding: crate::daemon::ability::dispatch::CatalogRuntimeBinding {
                state: crate::daemon::ability::dispatch::CatalogRuntimeBindingState::Bound,
                implementation_source: "test".to_string(),
                runtime_env: "daemon-native".to_string(),
            },
        };

        assert!(
            AbilityPublication::from_catalog_row(row).is_none(),
            "publication must fail closed when AbilityImplBinding targets a different handler key"
        );
    }

    #[test]
    fn publication_row_rejects_system_agent_callee_as_execution_host() {
        let device_profile_owner_ura = "easynet:///r/acme/device/node-a";
        let system_agent_owner_ura =
            crate::core::ura::device_agent_ura("acme", "node-a", "runtime-introspection");
        let descriptor = AbilityDescriptor::new(
            "runtime.cursor",
            system_agent_owner_ura.clone(),
            crate::daemon::ability::descriptors::Visibility::Scoped,
            crate::daemon::ability::descriptors::AdmissionAction::Invoke,
        )
        .expect("test descriptor");
        let authority_scope = crate::daemon::ability::AuthorityScope::new(
            "system-agent:runtime-introspection",
            system_agent_owner_ura.clone(),
        )
        .expect("test authority scope");
        let authority = crate::daemon::ability::AuthorityBinding::local_self_for_descriptor(
            "runtime.cursor",
            authority_scope,
            &descriptor,
        )
        .expect("test authority binding");
        let implementation = crate::daemon::ability::AbilityImplBinding::new(
            "runtime.cursor",
            descriptor.version.clone(),
            descriptor.call_mode(),
            crate::daemon::ability::RuntimeEnv::daemon_native(),
            crate::daemon::ability::AbilityImplSource::Test,
        )
        .expect("test implementation binding");
        let row = crate::daemon::ability::dispatch::AuthorityAbilityCatalogSnapshotRow {
            name: "runtime.cursor".to_string(),
            owner: OwnerKind::runtime_introspection_system(),
            execution_host_ura: system_agent_owner_ura,
            descriptor,
            authority,
            implementation,
            runtime_binding: crate::daemon::ability::dispatch::CatalogRuntimeBinding {
                state: crate::daemon::ability::dispatch::CatalogRuntimeBindingState::Bound,
                implementation_source: "test".to_string(),
                runtime_env: "daemon-native".to_string(),
            },
        };

        assert!(
            AbilityPublication::from_catalog_row(row).is_none(),
            "SystemAgent callee must not be published as its own execution host; the Device hosts it"
        );
        assert!(
            OwnerKind::runtime_introspection_system().execution_host_matches_owner_ura(
                &crate::core::ura::device_agent_ura("acme", "node-a", "runtime-introspection"),
                device_profile_owner_ura,
            ),
            "matching sponsoring Device remains the valid execution host"
        );
    }

    #[test]
    fn publication_row_rejects_service_callee_as_execution_host() {
        let device_profile_owner_ura = "easynet:///r/acme/device/node-a";
        let pages_service_owner_ura = crate::core::ura::service_ura("acme", "alice", "pages");
        let descriptor = AbilityDescriptor::new(
            "project_list",
            pages_service_owner_ura.clone(),
            crate::daemon::ability::descriptors::Visibility::Scoped,
            crate::daemon::ability::descriptors::AdmissionAction::Invoke,
        )
        .expect("test descriptor");
        let owner = OwnerKind::Service {
            principal_id: "alice".to_string(),
            service_id: "pages".to_string(),
        };
        let authority_scope = crate::daemon::ability::AuthorityScope::new(
            "service:alice.pages",
            pages_service_owner_ura.clone(),
        )
        .expect("test authority scope");
        let authority = crate::daemon::ability::AuthorityBinding::local_self_for_descriptor(
            "project_list",
            authority_scope,
            &descriptor,
        )
        .expect("test authority binding");
        let implementation = crate::daemon::ability::AbilityImplBinding::new(
            "project_list",
            descriptor.version.clone(),
            descriptor.call_mode(),
            crate::daemon::ability::RuntimeEnv::daemon_native(),
            crate::daemon::ability::AbilityImplSource::Test,
        )
        .expect("test implementation binding");
        let row = crate::daemon::ability::dispatch::AuthorityAbilityCatalogSnapshotRow {
            name: "project_list".to_string(),
            owner: owner.clone(),
            execution_host_ura: pages_service_owner_ura.clone(),
            descriptor,
            authority,
            implementation,
            runtime_binding: crate::daemon::ability::dispatch::CatalogRuntimeBinding {
                state: crate::daemon::ability::dispatch::CatalogRuntimeBindingState::Bound,
                implementation_source: "test".to_string(),
                runtime_env: "daemon-native".to_string(),
            },
        };

        assert!(
            AbilityPublication::from_catalog_row(row).is_none(),
            "Service callee must not be published as its own execution host; a Device hosts the Service surface implementation"
        );
        assert!(
            owner.execution_host_matches_owner_ura(
                &pages_service_owner_ura,
                device_profile_owner_ura,
            ),
            "same-realm Device remains a valid local execution host for Service-owned publication"
        );
    }

    #[test]
    fn publication_snapshot_excludes_descriptor_only_rows_from_callable_projection() {
        let device_profile_owner_ura = "easynet:///r/acme/device/node-a";
        let hosted_agent_owner_ura = crate::core::ura::agent_ura("acme", "alice", "agent");
        let catalog = AxonAbilityCatalog::new_with_runtime_and_authority_context(
            crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
                crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
                None,
            ),
            AbilityAuthorityContext::for_device_authority_root_with_hosted_agents(
                device_profile_owner_ura,
                [hosted_agent_owner_ura.clone()],
            )
            .expect("device authority context"),
        );
        let owner = OwnerKind::Agent("agent".to_string());
        let authority_scope =
            crate::daemon::ability::AuthorityScope::new("agent:agent", hosted_agent_owner_ura)
                .expect("hosted Agent authority scope");
        catalog
            .hot_register_descriptor_only_with_authority_scope(
                "agent.declared-only",
                owner,
                authority_scope,
                crate::daemon::ability::manifest::AbilityManifest::new(
                    "declared-only",
                    "Discoverable declaration without an executor.",
                    serde_json::json!({"type": "object"}),
                )
                .and_then(|manifest| manifest.with_admission_action("read"))
                .expect("descriptor-only manifest"),
            )
            .expect("descriptor-only catalog registration");

        let row = catalog
            .authority_ability_catalog_snapshot()
            .into_iter()
            .find(|row| row.name == "agent.declared-only")
            .expect("descriptor-only row is still visible in the control-plane catalog");
        assert_eq!(
            row.runtime_binding.state,
            crate::daemon::ability::dispatch::CatalogRuntimeBindingState::DescriptorOnly
        );
        assert!(
            !LocalAbilityPublicationSnapshot::capture(&catalog)
                .resolves("easynet:///r/acme/agent/alice.agent", "declared-only"),
            "descriptor-only rows must not enter the callable local publication projection"
        );
    }

    #[test]
    fn publication_row_rejects_unbound_runtime_binding() {
        let device_profile_owner_ura = "easynet:///r/acme/device/node-a";
        let system_agent_owner_ura =
            crate::core::ura::device_agent_ura("acme", "node-a", "runtime-introspection");
        let descriptor = AbilityDescriptor::new(
            "runtime.cursor",
            system_agent_owner_ura.clone(),
            crate::daemon::ability::descriptors::Visibility::Scoped,
            crate::daemon::ability::descriptors::AdmissionAction::Invoke,
        )
        .expect("test descriptor");
        let authority_scope = crate::daemon::ability::AuthorityScope::new(
            "system-agent:runtime-introspection",
            system_agent_owner_ura,
        )
        .expect("test authority scope");
        let authority = crate::daemon::ability::AuthorityBinding::local_self_for_descriptor(
            "runtime.cursor",
            authority_scope,
            &descriptor,
        )
        .expect("test authority binding");
        let implementation = crate::daemon::ability::AbilityImplBinding::new(
            "runtime.cursor",
            descriptor.version.clone(),
            descriptor.call_mode(),
            crate::daemon::ability::RuntimeEnv::daemon_native(),
            crate::daemon::ability::AbilityImplSource::Test,
        )
        .expect("test implementation binding");
        let row = crate::daemon::ability::dispatch::AuthorityAbilityCatalogSnapshotRow {
            name: "runtime.cursor".to_string(),
            owner: OwnerKind::runtime_introspection_system(),
            execution_host_ura: device_profile_owner_ura.to_string(),
            descriptor,
            authority,
            implementation,
            runtime_binding: crate::daemon::ability::dispatch::CatalogRuntimeBinding {
                state: crate::daemon::ability::dispatch::CatalogRuntimeBindingState::Unbound,
                implementation_source: "test".to_string(),
                runtime_env: "daemon-native".to_string(),
            },
        };

        assert!(
            AbilityPublication::from_catalog_row(row).is_none(),
            "unbound rows must not become callable AbilityPublication records"
        );
    }

    #[test]
    fn dynamic_catalog_commit_does_not_report_success_when_publication_fence_fails() {
        let device_profile_owner_ura = "easynet:///r/acme/device/node-a";
        let system_agent_owner_ura =
            crate::core::ura::device_agent_ura("acme", "node-a", "runtime-introspection");
        let runtime = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        );
        let catalog = AxonAbilityCatalog::new_with_runtime_and_authority_context(
            Arc::clone(&runtime),
            AbilityAuthorityContext::for_device_authority_root(device_profile_owner_ura)
                .expect("device authority context"),
        );
        catalog
            .register_dynamic_publication_participant(
                Arc::new(|_| anyhow::bail!("durable publication fence unavailable")),
                Arc::new(|| panic!("failed prepare must not emit a commit notification")),
            )
            .unwrap();

        let error = catalog
            .hot_register_rpc_with_spec(
                "runtime.fenced",
                OwnerKind::runtime_introspection_system(),
                crate::daemon::ability::manifest::AbilityManifest::new(
                    "dynamic",
                    "test fallible publication fence",
                    serde_json::json!({"type": "object"}),
                )
                .and_then(|manifest| manifest.with_admission_action("invoke"))
                .expect("test manifest"),
                Arc::new(|_args| Ok(serde_json::json!({"ok": true}))),
            )
            .expect_err("catalog mutation must surface a failed durable publication fence");

        assert!(error
            .to_string()
            .contains("durable publication fence unavailable"));
        assert!(!catalog.has_dynamic("runtime.fenced"));
        assert!(!LocalAbilityPublicationSnapshot::capture(&catalog)
            .resolves(&system_agent_owner_ura, "runtime.fenced"));
        let runtime_key = crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(
            &system_agent_owner_ura,
            "runtime.fenced",
        )
        .expect("test runtime key");
        let runtime_row = crate::support::async_bridge::run_blocking(
            runtime.ability_options(&runtime_key),
            crate::support::async_bridge::SyncBridgeRuntimePolicy::BuildCurrentThreadTokio,
        );
        assert!(runtime_row.is_none());
    }

    #[test]
    fn dynamic_catalog_accepts_exactly_one_publication_coordinator() {
        let catalog = AxonAbilityCatalog::new_test_metadata_for_device_authority(
            "easynet:///r/acme/device/node-a",
        );
        catalog
            .register_dynamic_publication_participant(Arc::new(|_| Ok(())), Arc::new(|| {}))
            .expect("first coordinator owns the complete two-phase boundary");

        let error = catalog
            .register_dynamic_publication_participant(Arc::new(|_| Ok(())), Arc::new(|| {}))
            .expect_err("a second coordinator would reintroduce partial prepare");

        assert!(error.to_string().contains("already registered"));
    }

    #[test]
    fn dynamic_unregister_prepare_failure_leaves_catalog_index_and_runtime_unchanged() {
        let device_profile_owner_ura = "easynet:///r/acme/device/node-a";
        let system_agent_owner_ura =
            crate::core::ura::device_agent_ura("acme", "node-a", "runtime-introspection");
        let runtime = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        );
        let catalog = AxonAbilityCatalog::new_with_runtime_and_authority_context(
            Arc::clone(&runtime),
            AbilityAuthorityContext::for_device_authority_root(device_profile_owner_ura)
                .expect("device authority context"),
        );
        let fail_prepare = Arc::new(std::sync::atomic::AtomicBool::new(false));
        catalog
            .register_dynamic_publication_participant(
                Arc::new({
                    let fail_prepare = Arc::clone(&fail_prepare);
                    move |_| {
                        if fail_prepare.load(std::sync::atomic::Ordering::SeqCst) {
                            anyhow::bail!("unregister publication fence unavailable")
                        }
                        Ok(())
                    }
                }),
                Arc::new(|| {}),
            )
            .unwrap();
        catalog
            .hot_register_rpc_with_spec(
                "runtime.kept",
                OwnerKind::runtime_introspection_system(),
                crate::daemon::ability::manifest::AbilityManifest::new(
                    "dynamic",
                    "test unregister prepare fence",
                    serde_json::json!({"type": "object"}),
                )
                .and_then(|manifest| manifest.with_admission_action("invoke"))
                .expect("test manifest"),
                Arc::new(|_args| Ok(serde_json::json!({"ok": true}))),
            )
            .expect("initial registration");
        fail_prepare.store(true, std::sync::atomic::Ordering::SeqCst);

        let error = catalog
            .hot_unregister("runtime.kept")
            .expect_err("failed prepare must stop unregister before local mutation");
        assert!(error
            .to_string()
            .contains("unregister publication fence unavailable"));
        assert!(catalog.has_dynamic("runtime.kept"));
        assert!(LocalAbilityPublicationSnapshot::capture(&catalog)
            .resolves(&system_agent_owner_ura, "runtime.kept"));
        let runtime_key = crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(
            &system_agent_owner_ura,
            "runtime.kept",
        )
        .expect("test runtime key");
        let runtime_row = crate::support::async_bridge::run_blocking(
            runtime.ability_options(&runtime_key),
            crate::support::async_bridge::SyncBridgeRuntimePolicy::BuildCurrentThreadTokio,
        );
        assert!(runtime_row.is_some());
    }

    #[test]
    fn owner_projection_values_rejects_corrupt_committed_descriptor() {
        let owner_ura = "easynet:///r/acme/agent/device.node-a.runtime-introspection";
        let mut descriptor = AbilityDescriptor::new(
            "device_profile.cursor",
            owner_ura,
            crate::daemon::ability::descriptors::Visibility::Scoped,
            crate::daemon::ability::descriptors::AdmissionAction::Invoke,
        )
        .expect("test descriptor");
        let authority_scope =
            crate::daemon::ability::AuthorityScope::new("device", owner_ura).unwrap();
        let authority_binding =
            crate::daemon::ability::AuthorityBinding::local_self_for_descriptor(
                "device_profile.cursor",
                authority_scope,
                &descriptor,
            )
            .unwrap();
        let implementation_binding = crate::daemon::ability::AbilityImplBinding::new(
            "device_profile.cursor",
            descriptor.version.clone(),
            descriptor.call_mode(),
            crate::daemon::ability::RuntimeEnv::daemon_native(),
            crate::daemon::ability::AbilityImplSource::Test,
        )
        .unwrap();
        descriptor.owner_ura = "not-a-canonical-owner".to_string();
        let publication = AbilityPublication {
            descriptor_contract: descriptor,
            owner_binding: OwnerBinding {
                owner: OwnerKind::DeviceProfileProjection,
                owner_ura: "not-a-canonical-owner".to_string(),
                ability_ura:
                    "easynet:///r/acme/ability/device.node-a.runtime-introspection.device_profile.cursor"
                        .to_string(),
            },
            authority_binding,
            implementation_binding,
            route_binding: PublicationRouteBinding {
                callee_ura: "not-a-canonical-owner".to_string(),
                execution_host_ura: owner_ura.to_string(),
                descriptor_ref: "test-descriptor-ref".to_string(),
                dispatch_key: "device_profile.cursor".to_string(),
            },
        };
        let mut snapshot = LocalAbilityPublicationSnapshot::default();
        snapshot
            .publications_by_owner
            .entry("not-a-canonical-owner".to_string())
            .or_default()
            .push(publication);

        let err = snapshot
            .owner_projection_values("not-a-canonical-owner")
            .expect_err("corrupt committed descriptor must not be hidden as empty publication");
        assert!(
            err.contains("local ability publication") && err.contains("cannot derive ability URA"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn hosted_agent_projection_matches_canonical_agent_id_only() {
        let alice = crate::core::ura::agent_ura("acme", "user-1", "alice");
        let bob = crate::core::ura::agent_ura("acme", "user-1", "bob");
        let mut snapshot =
            LocalAbilityPublicationSnapshot::from_owner_public_names(&alice, &["chat", "search"]);
        let bob_snapshot =
            LocalAbilityPublicationSnapshot::from_owner_public_names(&bob, &["chat"]);
        snapshot
            .publications_by_owner
            .extend(bob_snapshot.publications_by_owner);

        let projected = snapshot.hosted_agent_descriptors("alice");
        assert_eq!(projected.len(), 2);
        assert!(projected
            .iter()
            .all(|descriptor| descriptor.owner_ura == alice));
    }

    #[test]
    fn hosted_agent_owner_uras_are_canonical_agent_owners_only() {
        let device = crate::core::ura::device_ura("acme", "device-1");
        let device_agent = crate::core::ura::agent_ura("acme", "device.device-1", "mcp-default");
        let alice = crate::core::ura::agent_ura("acme", "user-1", "alice");
        let bob = crate::core::ura::agent_ura("acme", "user-1", "bob");
        let mut snapshot = LocalAbilityPublicationSnapshot::default();
        snapshot.publications_by_owner.entry(device).or_default();
        snapshot.publications_by_owner.extend(
            LocalAbilityPublicationSnapshot::from_owner_public_names(
                &device_agent,
                &["mcp-default.search"],
            )
            .publications_by_owner,
        );
        snapshot.publications_by_owner.extend(
            LocalAbilityPublicationSnapshot::from_owner_public_names(&bob, &["chat"])
                .publications_by_owner,
        );
        snapshot.publications_by_owner.extend(
            LocalAbilityPublicationSnapshot::from_owner_public_names(&alice, &["chat"])
                .publications_by_owner,
        );
        snapshot
            .publications_by_owner
            .entry("not-a-ura".to_string())
            .or_default();

        assert_eq!(snapshot.hosted_agent_owner_uras(), vec![alice, bob]);
    }

    #[test]
    fn system_agent_owner_uras_are_kept_separate_from_hosted_user_agents() {
        let plugin_management =
            crate::core::ura::device_agent_ura("acme", "node-a", "plugin-management");
        let ability_management =
            crate::core::ura::device_agent_ura("acme", "node-a", "ability-management");
        let hosted = crate::core::ura::agent_ura("acme", "alice", "worker");
        let mut descriptors = Vec::new();
        for (owner, name) in [
            (&plugin_management, "plugin.echo"),
            (&ability_management, "ability.deploy"),
            (&hosted, "chat"),
        ] {
            descriptors.push(
                AbilityDescriptor::new(
                    name,
                    owner,
                    crate::daemon::ability::descriptors::Visibility::Scoped,
                    crate::daemon::ability::descriptors::AdmissionAction::Invoke,
                )
                .expect("descriptor"),
            );
        }
        let snapshot = LocalAbilityPublicationSnapshot::from_descriptors(descriptors);

        assert_eq!(
            snapshot.system_agent_owner_uras(),
            vec![ability_management, plugin_management]
        );
        assert_eq!(snapshot.hosted_agent_owner_uras(), vec![hosted]);
    }

    #[test]
    fn hosted_agent_projection_fails_closed_for_ambiguous_owner_ids() {
        let first = crate::core::ura::agent_ura("acme", "user-1", "alice");
        let second = crate::core::ura::agent_ura("acme", "user-2", "alice");
        let mut snapshot =
            LocalAbilityPublicationSnapshot::from_owner_public_names(&first, &["chat"]);
        let duplicate =
            LocalAbilityPublicationSnapshot::from_owner_public_names(&second, &["search"]);
        snapshot
            .publications_by_owner
            .extend(duplicate.publications_by_owner);

        assert!(snapshot.hosted_agent_descriptors("alice").is_empty());
    }
}
