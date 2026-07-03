// EasyNet CLI — Runtime trust invalidation side effects
// =====================================================
//
// File: src/daemon/invocation/runtime_trust_invalidator.rs
// Description: Runtime-side invalidation adapter for successful
//              identity trust mutations.
//
// Protocol Responsibility:
// This module does not change Axon admission or trust persistence.
// It projects a completed daemon trust mutation onto daemon-owned
// liveness/read-model state.
//
// Implementation Approach:
// Reuse the existing `PresenceRegistry` and `AdvertisedAgentStore`
// lifecycle surfaces. Do not add a second presence map or a parallel
// terminal-state vocabulary.
//
// Usage Contract:
// Call only after the trust writer has committed and published the new
// trust anchor. Idempotent no-op mutations must not emit offline events.
//
// Architectural Position:
// EasyNet-Cli daemon runtime adapter. Backend requests the mutation;
// daemon owns the runtime side effects.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::daemon::boot::join_connection_state::{
    record_snapshot, JoinConnectionSnapshot, JoinConnectionState, JoinTransition,
};
use crate::daemon::federation::read_model::advertised_agents::AdvertisedAgentStore;
use crate::daemon::invocation::bidi::state::presence::PresenceRegistry;
use crate::daemon::persistence::config::Credentials;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RuntimeTrustInvalidation {
    pub(crate) direct_presence_removed: bool,
    pub(crate) hosted_agents_removed: usize,
    pub(crate) hosted_hosts_revoked: usize,
    pub(crate) connection_state_recorded: bool,
}

impl RuntimeTrustInvalidation {
    #[must_use]
    pub(crate) fn no_op() -> Self {
        Self {
            direct_presence_removed: false,
            hosted_agents_removed: 0,
            hosted_hosts_revoked: 0,
            connection_state_recorded: false,
        }
    }

    #[must_use]
    pub(crate) fn removed_any_presence(self) -> bool {
        self.direct_presence_removed || self.hosted_hosts_revoked > 0
    }

    #[must_use]
    pub(crate) fn removed_any_hosted_agent(self) -> bool {
        self.hosted_agents_removed > 0
    }
}

#[derive(Clone)]
pub(crate) struct RuntimeTrustConnectionStateProjector {
    current_user_ura: String,
    credentials: Credentials,
    source: String,
}

impl RuntimeTrustConnectionStateProjector {
    #[must_use]
    pub(crate) fn from_local_credentials(source: impl Into<String>) -> Option<Self> {
        let credentials = crate::daemon::persistence::config::load_credentials().ok()?;
        Self::from_credentials(credentials, source)
    }

    #[must_use]
    pub(crate) fn from_credentials(
        credentials: Credentials,
        source: impl Into<String>,
    ) -> Option<Self> {
        let current_user_ura = credentials.user_ura().ok()?;
        Some(Self {
            current_user_ura,
            credentials,
            source: source.into(),
        })
    }

    fn record_disconnected_removed(
        &self,
        subject_ura: &str,
        invalidation: &RuntimeTrustInvalidation,
    ) -> bool {
        if !invalidation.direct_presence_removed || subject_ura != self.current_user_ura {
            return false;
        }
        record_snapshot(JoinConnectionSnapshot::from_credentials(
            JoinConnectionState::DisconnectedRemoved,
            Some(JoinTransition::RemovePresence),
            &self.credentials,
            self.source.clone(),
        ));
        true
    }
}

#[derive(Clone)]
pub(crate) struct RuntimeTrustInvalidator {
    presence: Arc<PresenceRegistry>,
    advertised_agents: Arc<AdvertisedAgentStore>,
    connection_state_projector: Option<RuntimeTrustConnectionStateProjector>,
}

impl RuntimeTrustInvalidator {
    #[must_use]
    pub(crate) fn new(
        presence: Arc<PresenceRegistry>,
        advertised_agents: Arc<AdvertisedAgentStore>,
    ) -> Self {
        Self {
            presence,
            advertised_agents,
            connection_state_projector: None,
        }
    }

    #[must_use]
    pub(crate) fn with_connection_state_projector(
        mut self,
        projector: Option<RuntimeTrustConnectionStateProjector>,
    ) -> Self {
        self.connection_state_projector = projector;
        self
    }

    /// Invalidate daemon runtime state for a successfully-revoked trust
    /// subject. Direct presence removal is key-aware when the revoked
    /// public key is supplied; hosted-agent host removal still follows
    /// the owner linkage read model.
    pub(crate) fn invalidate_revoked_subject(
        &self,
        subject_ura: &str,
        revoked_public_key_b64: Option<&str>,
        trust_row_removed: bool,
    ) -> RuntimeTrustInvalidation {
        if !trust_row_removed {
            return RuntimeTrustInvalidation::no_op();
        }

        let direct_presence_removed = match revoked_public_key_b64 {
            Some(public_key_b64) => self
                .presence
                .force_revoke_if_admitted_key(subject_ura, public_key_b64)
                .is_some(),
            None => self.presence.force_revoke(subject_ura).is_some(),
        };
        let mut removed_agents = Vec::new();
        if let Some(record) = self.advertised_agents.remove(subject_ura) {
            removed_agents.push(record);
        }
        removed_agents.extend(self.advertised_agents.remove_user_owned_agents(subject_ura));

        let host_uras: BTreeSet<String> = removed_agents
            .iter()
            .filter_map(|record| record.host_ura().map(str::to_string))
            .collect();
        let hosted_hosts_revoked = host_uras
            .iter()
            .filter(|host_ura| self.presence.force_revoke(host_ura).is_some())
            .count();

        let mut invalidation = RuntimeTrustInvalidation {
            direct_presence_removed,
            hosted_agents_removed: removed_agents.len(),
            hosted_hosts_revoked,
            connection_state_recorded: false,
        };
        if let Some(projector) = &self.connection_state_projector {
            invalidation.connection_state_recorded =
                projector.record_disconnected_removed(subject_ura, &invalidation);
        }
        invalidation
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::commands::test_support::HomeGuard;
    use crate::daemon::boot::join_connection_state::load_snapshot;
    use crate::daemon::federation::read_model::advertised_agents::{
        AdvertisedAgentRecord, AdvertisedAgentSigningAuthority,
    };
    use crate::daemon::invocation::bidi::state::presence::DISPATCH_CHANNEL_CAPACITY;
    use crate::daemon::persistence::config::Credentials;

    fn sender() -> crate::daemon::invocation::bidi::state::presence::DispatchSender {
        let (tx, _rx) = tokio::sync::mpsc::channel(DISPATCH_CHANNEL_CAPACITY);
        tx
    }

    fn credentials(username: &str) -> Credentials {
        Credentials {
            node_id: "dev-1".to_string(),
            credential_token: "credential-token".to_string(),
            hub_endpoint: "https://hub.local:50443".to_string(),
            realm: "local".to_string(),
            deploy_signature: String::new(),
            hub_api_base: Some("https://hub.local".to_string()),
            username: Some(username.to_string()),
            user_id: Some(format!("user-{username}")),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        }
    }

    #[test]
    fn idempotent_revoke_does_not_remove_presence() {
        let presence = Arc::new(PresenceRegistry::new());
        let advertised = Arc::new(AdvertisedAgentStore::new());
        let subject = "easynet:///r/local/user/user-1";
        presence.insert(subject.to_string(), sender());

        let invalidator = RuntimeTrustInvalidator::new(Arc::clone(&presence), advertised);
        let outcome = invalidator.invalidate_revoked_subject(subject, Some("pubkey-a"), false);

        assert_eq!(outcome, RuntimeTrustInvalidation::no_op());
        assert!(presence.contains(subject));
    }

    #[test]
    fn removed_revoke_force_revokes_direct_presence() {
        let presence = Arc::new(PresenceRegistry::new());
        let advertised = Arc::new(AdvertisedAgentStore::new());
        let subject = "easynet:///r/local/user/user-1";
        presence.insert_negotiated_with_trust(
            subject.to_string(),
            sender(),
            crate::daemon::invocation::bidi::state::presence::SessionContract::legacy(),
            crate::daemon::invocation::bidi::state::presence::SessionTrustContext::user_pubkey(
                "pubkey-a",
            ),
        );

        let invalidator = RuntimeTrustInvalidator::new(Arc::clone(&presence), advertised);
        let outcome = invalidator.invalidate_revoked_subject(subject, Some("pubkey-a"), true);

        assert!(outcome.direct_presence_removed);
        assert!(outcome.removed_any_presence());
        assert!(!presence.contains(subject));
    }

    #[test]
    fn removed_user_revoke_keeps_presence_admitted_by_different_key() {
        let presence = Arc::new(PresenceRegistry::new());
        let advertised = Arc::new(AdvertisedAgentStore::new());
        let subject = "easynet:///r/local/user/user-1";
        presence.insert_negotiated_with_trust(
            subject.to_string(),
            sender(),
            crate::daemon::invocation::bidi::state::presence::SessionContract::legacy(),
            crate::daemon::invocation::bidi::state::presence::SessionTrustContext::user_pubkey(
                "pubkey-b",
            ),
        );

        let invalidator = RuntimeTrustInvalidator::new(Arc::clone(&presence), advertised);
        let outcome = invalidator.invalidate_revoked_subject(subject, Some("pubkey-a"), true);

        assert!(!outcome.direct_presence_removed);
        assert!(!outcome.removed_any_presence());
        assert!(presence.contains(subject));
    }

    #[test]
    fn removed_local_user_revoke_records_disconnected_removed_snapshot() {
        let _home = HomeGuard::new();
        let presence = Arc::new(PresenceRegistry::new());
        let advertised = Arc::new(AdvertisedAgentStore::new());
        let subject = crate::core::ura::user_ura("local", "user-alice");
        presence.insert_negotiated_with_trust(
            subject.clone(),
            sender(),
            crate::daemon::invocation::bidi::state::presence::SessionContract::legacy(),
            crate::daemon::invocation::bidi::state::presence::SessionTrustContext::user_pubkey(
                "pubkey-a",
            ),
        );

        let projector =
            RuntimeTrustConnectionStateProjector::from_credentials(credentials("alice"), "test")
                .expect("projector from complete credentials");
        let invalidator = RuntimeTrustInvalidator::new(Arc::clone(&presence), advertised)
            .with_connection_state_projector(Some(projector));
        let outcome = invalidator.invalidate_revoked_subject(&subject, Some("pubkey-a"), true);

        assert!(outcome.direct_presence_removed);
        assert!(outcome.connection_state_recorded);
        let snapshot = load_snapshot().expect("snapshot recorded");
        assert_eq!(snapshot.state, "OFFLINE");
        assert_eq!(snapshot.state_code, "F530");
        assert_eq!(
            snapshot.transition_id.as_deref(),
            Some("T12_REMOVE_PRESENCE")
        );
        assert_eq!(
            snapshot.device_ura,
            crate::core::ura::device_ura("local", "dev-1")
        );
        assert_eq!(snapshot.source, "test");
    }

    #[test]
    fn removed_non_local_user_revoke_does_not_record_connection_state() {
        let _home = HomeGuard::new();
        let presence = Arc::new(PresenceRegistry::new());
        let advertised = Arc::new(AdvertisedAgentStore::new());
        let subject = crate::core::ura::user_ura("local", "user-bob");
        presence.insert_negotiated_with_trust(
            subject.clone(),
            sender(),
            crate::daemon::invocation::bidi::state::presence::SessionContract::legacy(),
            crate::daemon::invocation::bidi::state::presence::SessionTrustContext::user_pubkey(
                "pubkey-a",
            ),
        );

        let projector =
            RuntimeTrustConnectionStateProjector::from_credentials(credentials("alice"), "test")
                .expect("projector from complete credentials");
        let invalidator = RuntimeTrustInvalidator::new(Arc::clone(&presence), advertised)
            .with_connection_state_projector(Some(projector));
        let outcome = invalidator.invalidate_revoked_subject(&subject, Some("pubkey-a"), true);

        assert!(outcome.direct_presence_removed);
        assert!(!outcome.connection_state_recorded);
        assert!(
            load_snapshot().is_err(),
            "hub-side revoke for another user must not overwrite this machine's connection-state"
        );
    }

    #[test]
    fn removed_revoke_removes_hosted_agent_and_host_presence() {
        let presence = Arc::new(PresenceRegistry::new());
        let advertised = Arc::new(AdvertisedAgentStore::new());
        let subject = "easynet:///r/local/agent/alice.helper";
        let host = "easynet:///r/local/device/dev-1";
        presence.insert(host.to_string(), sender());
        advertised.upsert(AdvertisedAgentRecord {
            agent_ura: subject.to_string(),
            public_key_hex: String::new(),
            host_node_id: Some("dev-1".to_string()),
            signing_authority: AdvertisedAgentSigningAuthority::HostedBy {
                host_ura: host.to_string(),
            },
        });

        let invalidator =
            RuntimeTrustInvalidator::new(Arc::clone(&presence), Arc::clone(&advertised));
        let outcome = invalidator.invalidate_revoked_subject(subject, None, true);

        assert_eq!(outcome.hosted_agents_removed, 1);
        assert_eq!(outcome.hosted_hosts_revoked, 1);
        assert!(!presence.contains(host));
        assert!(advertised.get(subject).is_none());
    }

    #[test]
    fn removed_user_revoke_fans_out_to_owned_hosted_agents_and_dedupes_hosts() {
        let presence = Arc::new(PresenceRegistry::new());
        let advertised = Arc::new(AdvertisedAgentStore::new());
        let user = "easynet:///r/local/user/alice";
        let host = "easynet:///r/local/device/dev-1";
        presence.insert(host.to_string(), sender());
        for agent_ura in [
            "easynet:///r/local/agent/alice.helper",
            "easynet:///r/local/agent/alice.researcher",
        ] {
            advertised.upsert(AdvertisedAgentRecord {
                agent_ura: agent_ura.to_string(),
                public_key_hex: String::new(),
                host_node_id: Some("dev-1".to_string()),
                signing_authority: AdvertisedAgentSigningAuthority::HostedBy {
                    host_ura: host.to_string(),
                },
            });
        }
        advertised.upsert(AdvertisedAgentRecord {
            agent_ura: "easynet:///r/local/agent/bob.helper".to_string(),
            public_key_hex: String::new(),
            host_node_id: Some("dev-2".to_string()),
            signing_authority: AdvertisedAgentSigningAuthority::HostedBy {
                host_ura: "easynet:///r/local/device/dev-2".to_string(),
            },
        });

        let invalidator =
            RuntimeTrustInvalidator::new(Arc::clone(&presence), Arc::clone(&advertised));
        let outcome = invalidator.invalidate_revoked_subject(user, None, true);

        assert_eq!(outcome.hosted_agents_removed, 2);
        assert_eq!(
            outcome.hosted_hosts_revoked, 1,
            "one host device should be revoked once even when it owns multiple agents"
        );
        assert!(!presence.contains(host));
        assert!(advertised
            .get("easynet:///r/local/agent/alice.helper")
            .is_none());
        assert!(advertised
            .get("easynet:///r/local/agent/alice.researcher")
            .is_none());
        assert!(advertised
            .get("easynet:///r/local/agent/bob.helper")
            .is_some());
    }
}
