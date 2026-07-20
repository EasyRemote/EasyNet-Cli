//! Immutable publication view captured from the live ability control plane.
//!
//! Route admission and directory publication consume the same committed
//! `AxonAbilityCatalog` records. This prevents resolver listings from drifting
//! away from hot registrations that are already executable by `LocalRuntime`.

use std::collections::{BTreeMap, HashSet};

use serde_json::Value;

use crate::daemon::ability::descriptors::AbilityDescriptor;
use crate::daemon::ability::dispatch::AxonAbilityCatalog;

#[derive(Debug, Clone, Default)]
pub(crate) struct LocalAbilityPublicationSnapshot {
    descriptors_by_owner: BTreeMap<String, Vec<AbilityDescriptor>>,
    ability_uras: HashSet<String>,
}

impl LocalAbilityPublicationSnapshot {
    #[must_use]
    pub(crate) fn capture(catalog: &AxonAbilityCatalog) -> Self {
        let mut snapshot = Self::default();
        for row in catalog.authority_ability_catalog_snapshot() {
            if !super::is_local_runtime_routable_catalog_name(&row.name) {
                continue;
            }
            let descriptor = row.descriptor;
            let Some(ability_ura) = descriptor.canonical_ability_ura() else {
                continue;
            };
            snapshot.ability_uras.insert(ability_ura);
            snapshot
                .descriptors_by_owner
                .entry(descriptor.owner_ura.clone())
                .or_default()
                .push(descriptor);
        }
        snapshot
    }

    #[must_use]
    pub(crate) fn resolves(&self, owner_ura: &str, public_name: &str) -> bool {
        crate::core::ura::owner_ability_ura(owner_ura, public_name)
            .is_some_and(|ability_ura| self.ability_uras.contains(&ability_ura))
    }

    pub(crate) fn owner_projection_values(&self, owner_ura: &str) -> Result<Vec<Value>, String> {
        self.descriptors_by_owner
            .get(owner_ura)
            .into_iter()
            .flatten()
            .map(|descriptor| {
                crate::daemon::federation::read_model::owner_projection::summary_from_descriptor(
                    descriptor,
                )
                .map_err(|error| {
                    format!(
                        "local ability publication for owner `{owner_ura}` descriptor `{}` is invalid: {error}",
                        descriptor.name
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
        self.descriptors_by_owner
            .get(owner_ura)
            .cloned()
            .unwrap_or_default()
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
            .descriptors_by_owner
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
        let owner_descriptors = matching_owners[0].1;
        let mut descriptors = owner_descriptors.to_vec();
        descriptors.sort_by(|left, right| {
            left.public_name()
                .cmp(&right.public_name())
                .then_with(|| left.call_mode().as_str().cmp(right.call_mode().as_str()))
                .then_with(|| left.version.cmp(&right.version))
        });
        descriptors
    }

    #[must_use]
    pub(crate) fn all_descriptors(&self) -> Vec<AbilityDescriptor> {
        self.descriptors_by_owner
            .values()
            .flatten()
            .cloned()
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn from_owner_public_names(owner_ura: &str, public_names: &[&str]) -> Self {
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
            let ability_ura = descriptor
                .canonical_ability_ura()
                .expect("test canonical ability URA");
            snapshot.ability_uras.insert(ability_ura);
            snapshot
                .descriptors_by_owner
                .entry(owner_ura.to_string())
                .or_default()
                .push(descriptor);
        }
        snapshot
    }

    #[cfg(test)]
    pub(crate) fn from_descriptors(descriptors: Vec<AbilityDescriptor>) -> Self {
        let mut snapshot = Self::default();
        for descriptor in descriptors {
            let ability_ura = descriptor
                .canonical_ability_ura()
                .expect("test descriptor must derive a canonical Ability URA");
            snapshot.ability_uras.insert(ability_ura);
            snapshot
                .descriptors_by_owner
                .entry(descriptor.owner_ura.clone())
                .or_default()
                .push(descriptor);
        }
        snapshot
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::daemon::ability::dispatch::{AbilityAuthorityContext, OwnerKind};

    #[test]
    fn capture_tracks_hot_control_plane_commits_without_mutating_prior_snapshot() {
        let owner_ura = "easynet:///r/acme/device/node-a";
        let catalog = AxonAbilityCatalog::new_with_runtime_and_authority_context(
            crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
                crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
                None,
            ),
            AbilityAuthorityContext::for_device_authority_root(owner_ura)
                .expect("device authority context"),
        );
        let before = LocalAbilityPublicationSnapshot::capture(&catalog);

        catalog
            .hot_register_rpc_with_spec(
                "plugin.dynamic",
                OwnerKind::Device,
                crate::daemon::ability::manifest::AbilityManifest::new(
                    "dynamic",
                    "test dynamic ability",
                    serde_json::json!({"type": "object"}),
                )
                .and_then(|manifest| manifest.with_admission_action("invoke"))
                .expect("test manifest"),
                Arc::new(|_args| Ok(serde_json::json!({"ok": true}))),
            )
            .expect("hot-register dynamic ability");
        let after = LocalAbilityPublicationSnapshot::capture(&catalog);

        assert!(!before.resolves(owner_ura, "plugin.dynamic"));
        assert!(after.resolves(owner_ura, "plugin.dynamic"));
        let published = after
            .owner_projection_values(owner_ura)
            .expect("local publication must project");
        assert!(published.iter().any(|summary| {
            summary.get("namespace").and_then(Value::as_str) == Some("plugin")
                && summary.get("local_name").and_then(Value::as_str) == Some("dynamic")
        }));
    }

    #[test]
    fn owner_projection_values_rejects_corrupt_committed_descriptor() {
        let owner_ura = "easynet:///r/acme/device/node-a";
        let mut descriptor = AbilityDescriptor::new(
            "plugin.dynamic",
            owner_ura,
            crate::daemon::ability::descriptors::Visibility::Scoped,
            crate::daemon::ability::descriptors::AdmissionAction::Invoke,
        )
        .expect("test descriptor");
        descriptor.owner_ura = "not-a-canonical-owner".to_string();
        let mut snapshot = LocalAbilityPublicationSnapshot::default();
        snapshot
            .descriptors_by_owner
            .entry("not-a-canonical-owner".to_string())
            .or_default()
            .push(descriptor);

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
            .descriptors_by_owner
            .extend(bob_snapshot.descriptors_by_owner);

        let projected = snapshot.hosted_agent_descriptors("alice");
        assert_eq!(projected.len(), 2);
        assert!(projected
            .iter()
            .all(|descriptor| descriptor.owner_ura == alice));
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
            .descriptors_by_owner
            .extend(duplicate.descriptors_by_owner);

        assert!(snapshot.hosted_agent_descriptors("alice").is_empty());
    }
}
