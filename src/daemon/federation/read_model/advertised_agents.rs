// EasyNet CLI — daemon federation read model — AdvertisedAgentStore
// =================================================================
//
// File: src/daemon/federation/read_model/advertised_agents.rs
//
// Why this exists
// ---------------
// `federation.advertise_agent` publishes hosted user agents whose
// online/offline bit actually lives on the host device's live
// `session.open`. PresenceRegistry alone therefore cannot answer
// "which `/agent/<user>.<agent>` rows should federation.resolve
// surface right now?" — it only knows about device URAs.
//
// This store keeps the host linkage:
//
//   hosted agent URA -> host device URA
//
// `federation.resolve` combines this with PresenceRegistry:
// hosted agents are active exactly when their host device URA is
// present. Self-signed agents fall back to their own URA.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::{Arc, Mutex, MutexGuard};

use dashmap::mapref::entry::Entry;
use dashmap::DashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdvertisedAgentSigningAuthority {
    SelfSigned,
    HostedBy { host_ura: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvertisedAgentRecord {
    pub agent_ura: String,
    pub generation: u64,
    pub public_key_hex: String,
    pub host_node_id: Option<String>,
    pub signing_authority: AdvertisedAgentSigningAuthority,
}

impl AdvertisedAgentRecord {
    #[must_use]
    pub fn host_ura(&self) -> Option<&str> {
        match &self.signing_authority {
            AdvertisedAgentSigningAuthority::SelfSigned => None,
            AdvertisedAgentSigningAuthority::HostedBy { host_ura } => Some(host_ura.as_str()),
        }
    }
}

impl From<crate::daemon::persistence::federation_revoke::HostedAgentInventoryRecord>
    for AdvertisedAgentRecord
{
    fn from(
        record: crate::daemon::persistence::federation_revoke::HostedAgentInventoryRecord,
    ) -> Self {
        let signing_authority = match record.signing_authority {
            crate::daemon::persistence::federation_revoke::DurableSigningAuthority::SelfSigned => {
                AdvertisedAgentSigningAuthority::SelfSigned
            }
            crate::daemon::persistence::federation_revoke::DurableSigningAuthority::HostedBy {
                host_ura,
            } => AdvertisedAgentSigningAuthority::HostedBy { host_ura },
        };
        Self {
            agent_ura: record.agent_ura,
            generation: record.generation,
            public_key_hex: record.public_key_hex,
            host_node_id: record.host_node_id,
            signing_authority,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AdvertisedAgentStore {
    inner: Arc<DashMap<String, AdvertisedAgentRecord>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdvertisedAgentUpsertOutcome {
    Inserted,
    AdvancedGeneration,
    Idempotent,
    IgnoredStale,
    RejectedConflict,
}

impl AdvertisedAgentUpsertOutcome {
    #[must_use]
    pub fn is_stored(self) -> bool {
        matches!(
            self,
            Self::Inserted | Self::AdvancedGeneration | Self::Idempotent
        )
    }
}

/// Linearization boundary for hosted-Agent lifecycle transitions.
///
/// Durable inventory, owner binding, identity read model, ability projection,
/// and revoke are one aggregate lifecycle. The transition is intentionally
/// process-wide at the Hub: management traffic is low-volume, while a global
/// gate prevents different code paths from observing a partially committed
/// generation.
#[derive(Debug, Default)]
pub struct HostedAgentLifecycleCoordinator {
    gate: Mutex<()>,
}

pub struct HostedAgentLifecycleTransition<'a> {
    _guard: MutexGuard<'a, ()>,
}

#[derive(Debug, thiserror::Error)]
pub enum HostedAgentLifecycleError {
    #[error("hosted-Agent lifecycle coordinator is poisoned after an incomplete transition")]
    Poisoned,
}

impl HostedAgentLifecycleCoordinator {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn begin(&self) -> Result<HostedAgentLifecycleTransition<'_>, HostedAgentLifecycleError> {
        let guard = self
            .gate
            .lock()
            .map_err(|_| HostedAgentLifecycleError::Poisoned)?;
        Ok(HostedAgentLifecycleTransition { _guard: guard })
    }
}

impl AdvertisedAgentStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert(&self, record: AdvertisedAgentRecord) -> AdvertisedAgentUpsertOutcome {
        match self.inner.entry(record.agent_ura.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(record);
                AdvertisedAgentUpsertOutcome::Inserted
            }
            Entry::Occupied(mut entry) => {
                let current = entry.get();
                if record.generation < current.generation {
                    return AdvertisedAgentUpsertOutcome::IgnoredStale;
                }
                if record.generation == current.generation {
                    return if current == &record {
                        AdvertisedAgentUpsertOutcome::Idempotent
                    } else {
                        AdvertisedAgentUpsertOutcome::RejectedConflict
                    };
                }
                entry.insert(record);
                AdvertisedAgentUpsertOutcome::AdvancedGeneration
            }
        }
    }

    pub fn get(&self, agent_ura: &str) -> Option<AdvertisedAgentRecord> {
        self.inner.get(agent_ura).map(|entry| entry.clone())
    }

    pub fn remove(&self, agent_ura: &str) -> Option<AdvertisedAgentRecord> {
        self.inner.remove(agent_ura).map(|(_, record)| record)
    }

    /// Compare-and-remove prevents a delayed revoke for an old incarnation
    /// from deleting a newly advertised row with the same URA.
    pub fn remove_generation(
        &self,
        agent_ura: &str,
        generation: u64,
    ) -> Option<AdvertisedAgentRecord> {
        self.inner
            .remove_if(agent_ura, |_ura, record| record.generation == generation)
            .map(|(_, record)| record)
    }

    /// Remove every advertised agent whose canonical owner is the supplied
    /// user subject URA.
    ///
    /// The linkage rule is Axon URA structure, not string-prefix matching:
    /// `easynet:///r/<realm>/user/<user>` owns
    /// `easynet:///r/<realm>/agent/<user>.<agent>`. Device-sponsored agent
    /// URAs intentionally do not match this method.
    pub fn remove_user_owned_agents(&self, user_ura: &str) -> Vec<AdvertisedAgentRecord> {
        let Ok(user) = crate::core::ura::parse_ura(user_ura) else {
            return Vec::new();
        };
        if user.kind != crate::core::ura::URAKind::User {
            return Vec::new();
        }
        let Some(user_id) = user.user_id() else {
            return Vec::new();
        };

        let keys: Vec<String> = self
            .inner
            .iter()
            .filter_map(|entry| {
                if record_is_owned_by_user(entry.value(), &user.realm, user_id) {
                    Some(entry.key().clone())
                } else {
                    None
                }
            })
            .collect();

        keys.into_iter()
            .filter_map(|key| self.remove(&key))
            .collect()
    }

    #[must_use]
    pub fn snapshot(&self) -> Vec<AdvertisedAgentRecord> {
        self.inner.iter().map(|entry| entry.clone()).collect()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

fn record_is_owned_by_user(record: &AdvertisedAgentRecord, realm: &str, user_id: &str) -> bool {
    let Ok(agent) = crate::core::ura::parse_ura(&record.agent_ura) else {
        return false;
    };
    agent.kind == crate::core::ura::URAKind::Agent
        && agent.realm == realm
        && agent.agent_ids().map(|(owner, _)| owner) == Some(user_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_and_get_round_trip() {
        let store = AdvertisedAgentStore::new();
        let record = AdvertisedAgentRecord {
            agent_ura: "easynet:///r/realm/agent/user.alice".into(),
            generation: 1,
            public_key_hex: String::new(),
            host_node_id: Some("dev-1".into()),
            signing_authority: AdvertisedAgentSigningAuthority::HostedBy {
                host_ura: "easynet:///r/realm/device/dev-1".into(),
            },
        };
        assert_eq!(
            store.upsert(record.clone()),
            AdvertisedAgentUpsertOutcome::Inserted
        );
        assert_eq!(store.get(&record.agent_ura), Some(record));
    }

    #[test]
    fn stale_generation_cannot_replace_current_host_route() {
        let store = AdvertisedAgentStore::new();
        let current = AdvertisedAgentRecord {
            agent_ura: "easynet:///r/realm/agent/user.alice".into(),
            generation: 2,
            public_key_hex: String::new(),
            host_node_id: Some("dev-2".into()),
            signing_authority: AdvertisedAgentSigningAuthority::HostedBy {
                host_ura: "easynet:///r/realm/device/dev-2".into(),
            },
        };
        let stale = AdvertisedAgentRecord {
            generation: 1,
            host_node_id: Some("dev-1".into()),
            signing_authority: AdvertisedAgentSigningAuthority::HostedBy {
                host_ura: "easynet:///r/realm/device/dev-1".into(),
            },
            ..current.clone()
        };
        assert!(store.upsert(current.clone()).is_stored());
        assert_eq!(
            store.upsert(stale),
            AdvertisedAgentUpsertOutcome::IgnoredStale
        );
        assert_eq!(store.get(&current.agent_ura), Some(current));
    }

    #[test]
    fn remove_deletes_row() {
        let store = AdvertisedAgentStore::new();
        let record = AdvertisedAgentRecord {
            agent_ura: "ura".into(),
            generation: 1,
            public_key_hex: String::new(),
            host_node_id: None,
            signing_authority: AdvertisedAgentSigningAuthority::SelfSigned,
        };
        store.upsert(record.clone());
        assert_eq!(store.remove("ura"), Some(record));
        assert!(store.get("ura").is_none());
    }

    #[test]
    fn remove_user_owned_agents_uses_canonical_agent_owner() {
        let store = AdvertisedAgentStore::new();
        let alice_agent = AdvertisedAgentRecord {
            agent_ura: "easynet:///r/realm/agent/alice.helper".into(),
            generation: 1,
            public_key_hex: String::new(),
            host_node_id: Some("dev-1".into()),
            signing_authority: AdvertisedAgentSigningAuthority::HostedBy {
                host_ura: "easynet:///r/realm/device/dev-1".into(),
            },
        };
        let alice_second = AdvertisedAgentRecord {
            agent_ura: "easynet:///r/realm/agent/alice.researcher".into(),
            generation: 1,
            public_key_hex: String::new(),
            host_node_id: Some("dev-1".into()),
            signing_authority: AdvertisedAgentSigningAuthority::HostedBy {
                host_ura: "easynet:///r/realm/device/dev-1".into(),
            },
        };
        let bob_agent = AdvertisedAgentRecord {
            agent_ura: "easynet:///r/realm/agent/bob.helper".into(),
            generation: 1,
            public_key_hex: String::new(),
            host_node_id: Some("dev-2".into()),
            signing_authority: AdvertisedAgentSigningAuthority::HostedBy {
                host_ura: "easynet:///r/realm/device/dev-2".into(),
            },
        };
        let other_realm_alice = AdvertisedAgentRecord {
            agent_ura: "easynet:///r/other/agent/alice.helper".into(),
            generation: 1,
            public_key_hex: String::new(),
            host_node_id: Some("dev-3".into()),
            signing_authority: AdvertisedAgentSigningAuthority::HostedBy {
                host_ura: "easynet:///r/other/device/dev-3".into(),
            },
        };
        for record in [
            alice_agent.clone(),
            alice_second.clone(),
            bob_agent.clone(),
            other_realm_alice.clone(),
        ] {
            store.upsert(record);
        }

        let mut removed = store.remove_user_owned_agents("easynet:///r/realm/user/alice");
        removed.sort_by(|a, b| a.agent_ura.cmp(&b.agent_ura));

        assert_eq!(removed, vec![alice_agent, alice_second]);
        assert!(store.get("easynet:///r/realm/agent/alice.helper").is_none());
        assert!(store
            .get("easynet:///r/realm/agent/alice.researcher")
            .is_none());
        assert_eq!(
            store.get("easynet:///r/realm/agent/bob.helper"),
            Some(bob_agent)
        );
        assert_eq!(
            store.get("easynet:///r/other/agent/alice.helper"),
            Some(other_realm_alice)
        );
    }

    #[test]
    fn remove_user_owned_agents_ignores_invalid_or_non_user_subject() {
        let store = AdvertisedAgentStore::new();
        let record = AdvertisedAgentRecord {
            agent_ura: "easynet:///r/realm/agent/alice.helper".into(),
            generation: 1,
            public_key_hex: String::new(),
            host_node_id: Some("dev-1".into()),
            signing_authority: AdvertisedAgentSigningAuthority::HostedBy {
                host_ura: "easynet:///r/realm/device/dev-1".into(),
            },
        };
        store.upsert(record.clone());

        assert!(store.remove_user_owned_agents("not-a-ura").is_empty());
        assert!(store
            .remove_user_owned_agents("easynet:///r/realm/device/dev-1")
            .is_empty());
        assert_eq!(store.get(&record.agent_ura), Some(record));
    }
}
