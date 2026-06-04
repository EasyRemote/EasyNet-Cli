// EasyNet CLI — Services Layer — AdvertisedAgentStore
// ====================================================
//
// File: src/services/advertised_agent_store.rs
//
// Why this exists
// ---------------
// `federation.advertise_agent` publishes hosted user agents whose
// online/offline bit actually lives on the host device's live
// `<self>.session`. PresenceRegistry alone therefore cannot answer
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

use std::sync::Arc;

use dashmap::DashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdvertisedAgentSigningAuthority {
    SelfSigned,
    HostedBy { host_ura: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvertisedAgentRecord {
    pub agent_ura: String,
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

#[derive(Debug, Clone, Default)]
pub struct AdvertisedAgentStore {
    inner: Arc<DashMap<String, AdvertisedAgentRecord>>,
}

impl AdvertisedAgentStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert(&self, record: AdvertisedAgentRecord) -> Option<AdvertisedAgentRecord> {
        self.inner.insert(record.agent_ura.clone(), record)
    }

    pub fn get(&self, agent_ura: &str) -> Option<AdvertisedAgentRecord> {
        self.inner.get(agent_ura).map(|entry| entry.clone())
    }

    pub fn remove(&self, agent_ura: &str) -> Option<AdvertisedAgentRecord> {
        self.inner.remove(agent_ura).map(|(_, record)| record)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_and_get_round_trip() {
        let store = AdvertisedAgentStore::new();
        let record = AdvertisedAgentRecord {
            agent_ura: "easynet:///r/realm/agent/user.alice".into(),
            public_key_hex: String::new(),
            host_node_id: Some("dev-1".into()),
            signing_authority: AdvertisedAgentSigningAuthority::HostedBy {
                host_ura: "easynet:///r/realm/device/dev-1".into(),
            },
        };
        assert!(store.upsert(record.clone()).is_none());
        assert_eq!(store.get(&record.agent_ura), Some(record));
    }

    #[test]
    fn remove_deletes_row() {
        let store = AdvertisedAgentStore::new();
        let record = AdvertisedAgentRecord {
            agent_ura: "ura".into(),
            public_key_hex: String::new(),
            host_node_id: None,
            signing_authority: AdvertisedAgentSigningAuthority::SelfSigned,
        };
        store.upsert(record.clone());
        assert_eq!(store.remove("ura"), Some(record));
        assert!(store.get("ura").is_none());
    }
}
