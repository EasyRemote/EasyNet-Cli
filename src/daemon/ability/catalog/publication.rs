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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalPublishedAbility {
    pub public_name: String,
    pub descriptor_ref: String,
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

    pub(crate) fn resolve(
        &self,
        owner_ura: &str,
        public_name: &str,
    ) -> Option<LocalPublishedAbility> {
        let ability_ura = crate::core::ura::owner_ability_ura(owner_ura, public_name)?;
        if !self.ability_uras.contains(&ability_ura) {
            return None;
        }
        let descriptor = self
            .descriptors_by_owner
            .get(owner_ura)?
            .iter()
            .find(|descriptor| {
                descriptor.public_name() == public_name
                    && descriptor.call_mode() == crate::daemon::ability::CallMode::Rpc
            })?;
        let descriptor_ref = descriptor.descriptor_ref().ok()?;
        Some(LocalPublishedAbility {
            public_name: public_name.to_string(),
            descriptor_ref,
        })
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) fn resolves(&self, owner_ura: &str, public_name: &str) -> bool {
        self.resolve(owner_ura, public_name).is_some()
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
        self.descriptors_by_owner
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
        self.descriptors_by_owner
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
            .descriptors_by_owner
            .iter()
            .filter_map(|(owner_ura, descriptors)| {
                let owner = crate::core::ura::parse_ura(owner_ura).ok()?;
                let (owner_device_id, _) = owner.device_agent_ids()?;
                if owner.realm != device.realm
                    || owner_device_id != device_id
                    || !descriptors
                        .iter()
                        .any(|descriptor| descriptor.public_name() == public_name)
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
    fn hosted_agent_owner_uras_are_canonical_agent_owners_only() {
        let device = crate::core::ura::device_ura("acme", "device-1");
        let device_agent = crate::core::ura::agent_ura("acme", "device.device-1", "mcp-default");
        let alice = crate::core::ura::agent_ura("acme", "user-1", "alice");
        let bob = crate::core::ura::agent_ura("acme", "user-1", "bob");
        let mut snapshot = LocalAbilityPublicationSnapshot::default();
        snapshot.descriptors_by_owner.entry(device).or_default();
        snapshot.descriptors_by_owner.extend(
            LocalAbilityPublicationSnapshot::from_owner_public_names(
                &device_agent,
                &["mcp-default.search"],
            )
            .descriptors_by_owner,
        );
        snapshot.descriptors_by_owner.extend(
            LocalAbilityPublicationSnapshot::from_owner_public_names(&bob, &["chat"])
                .descriptors_by_owner,
        );
        snapshot.descriptors_by_owner.extend(
            LocalAbilityPublicationSnapshot::from_owner_public_names(&alice, &["chat"])
                .descriptors_by_owner,
        );
        snapshot
            .descriptors_by_owner
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
            .descriptors_by_owner
            .extend(duplicate.descriptors_by_owner);

        assert!(snapshot.hosted_agent_descriptors("alice").is_empty());
    }
}
