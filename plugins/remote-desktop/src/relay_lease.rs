//! Hub-owned relay lease port for RemoteApp.
//!
//! Raw TURN credentials are transport representation only. Runtime Core still
//! owns Invocation authority, receipts, and terminal lifecycle; this module
//! owns the product session's bounded relay allocation.

use std::fmt;

use anyhow::bail;
use serde_json::{json, Value};

pub(in crate::daemon) const EASYNET_RELAY_PROVIDER: &str = "easynet_relay";

#[derive(Clone, PartialEq, Eq)]
pub(in crate::daemon) struct RemoteDesktopRelayLease {
    provider: String,
    lease_id: String,
    session_id: String,
    device_ura: String,
    resource_ura: String,
    urls: Vec<String>,
    username: String,
    credential: String,
    issued_at_ms: u64,
    expires_at_ms: u64,
    refresh_after_ms: u64,
}

impl fmt::Debug for RemoteDesktopRelayLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RemoteDesktopRelayLease")
            .field("provider", &self.provider)
            .field("lease_id", &self.lease_id)
            .field("session_id", &self.session_id)
            .field("device_ura", &self.device_ura)
            .field("resource_ura", &self.resource_ura)
            .field("urls", &self.urls)
            .field("username", &"<redacted>")
            .field("credential", &"<redacted>")
            .field("issued_at_ms", &self.issued_at_ms)
            .field("expires_at_ms", &self.expires_at_ms)
            .field("refresh_after_ms", &self.refresh_after_ms)
            .finish()
    }
}

pub(in crate::daemon) struct RemoteDesktopRelayLeaseInit {
    pub(in crate::daemon) provider: String,
    pub(in crate::daemon) lease_id: String,
    pub(in crate::daemon) session_id: String,
    pub(in crate::daemon) device_ura: String,
    pub(in crate::daemon) resource_ura: String,
    pub(in crate::daemon) urls: Vec<String>,
    pub(in crate::daemon) username: String,
    pub(in crate::daemon) credential: String,
    pub(in crate::daemon) issued_at_ms: u64,
    pub(in crate::daemon) expires_at_ms: u64,
    pub(in crate::daemon) refresh_after_ms: u64,
}

impl RemoteDesktopRelayLease {
    pub(in crate::daemon) fn from_init(
        expected_session_id: &str,
        expected_resource_ura: &str,
        init: RemoteDesktopRelayLeaseInit,
    ) -> anyhow::Result<Self> {
        if init.provider != EASYNET_RELAY_PROVIDER {
            bail!("Hub relay lease returned an unsupported provider");
        }
        if init.lease_id.trim().is_empty()
            || init.session_id != expected_session_id
            || init.resource_ura != expected_resource_ura
            || init.device_ura.trim().is_empty()
        {
            bail!("Hub relay lease identity does not match the RemoteApp session");
        }
        if init.urls.is_empty()
            || init.urls.iter().any(|url| {
                !(url.starts_with("turn:") || url.starts_with("turns:"))
                    || url.contains('@')
                    || url.chars().any(char::is_whitespace)
            })
            || init.username.trim().is_empty()
            || init.credential.trim().is_empty()
        {
            bail!("Hub relay lease contains invalid ICE server material");
        }
        if init.issued_at_ms == 0
            || init.refresh_after_ms <= init.issued_at_ms
            || init.expires_at_ms <= init.refresh_after_ms
        {
            bail!("Hub relay lease contains an invalid lifetime");
        }
        Ok(Self {
            provider: init.provider,
            lease_id: init.lease_id,
            session_id: init.session_id,
            device_ura: init.device_ura,
            resource_ura: init.resource_ura,
            urls: init.urls,
            username: init.username,
            credential: init.credential,
            issued_at_ms: init.issued_at_ms,
            expires_at_ms: init.expires_at_ms,
            refresh_after_ms: init.refresh_after_ms,
        })
    }

    pub(in crate::daemon) fn lease_id(&self) -> &str {
        &self.lease_id
    }

    pub(in crate::daemon) fn session_id(&self) -> &str {
        &self.session_id
    }

    pub(in crate::daemon) fn resource_ura(&self) -> &str {
        &self.resource_ura
    }

    pub(in crate::daemon) fn urls(&self) -> &[String] {
        &self.urls
    }

    pub(in crate::daemon) fn username(&self) -> &str {
        &self.username
    }

    pub(in crate::daemon) fn credential(&self) -> &str {
        &self.credential
    }

    pub(in crate::daemon) fn refresh_after_ms(&self) -> u64 {
        self.refresh_after_ms
    }

    pub(in crate::daemon) fn expires_at_ms(&self) -> u64 {
        self.expires_at_ms
    }

    fn evidence_value(&self) -> Value {
        json!({
            "provider": self.provider,
            "state": "active",
            "lease_id": self.lease_id,
            "session_id": self.session_id,
            "device_ura": self.device_ura,
            "resource_ura": self.resource_ura,
            "url_count": self.urls.len(),
            "ephemeral_auth_configured": true,
            "issued_at_ms": self.issued_at_ms,
            "expires_at_ms": self.expires_at_ms,
            "refresh_after_ms": self.refresh_after_ms,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon) enum RemoteDesktopRelayLeaseAvailability {
    Active(RemoteDesktopRelayLease),
    Unavailable { reason: String },
}

impl RemoteDesktopRelayLeaseAvailability {
    pub(in crate::daemon) fn unavailable(reason: impl Into<String>) -> Self {
        Self::Unavailable {
            reason: reason.into(),
        }
    }

    pub(in crate::daemon) fn active(&self) -> Option<&RemoteDesktopRelayLease> {
        match self {
            Self::Active(lease) => Some(lease),
            Self::Unavailable { .. } => None,
        }
    }

    pub(in crate::daemon) fn evidence_value(&self) -> Value {
        match self {
            Self::Active(lease) => lease.evidence_value(),
            Self::Unavailable { reason } => json!({
                "provider": EASYNET_RELAY_PROVIDER,
                "state": "unavailable",
                "reason": reason,
                "ephemeral_auth_configured": false,
            }),
        }
    }
}

pub(in crate::daemon) trait RemoteDesktopRelayLeaseProvider: Send + Sync {
    fn acquire(
        &self,
        session_id: &str,
        resource_ura: &str,
    ) -> anyhow::Result<RemoteDesktopRelayLeaseAvailability>;

    fn release(&self, lease: &RemoteDesktopRelayLease) -> anyhow::Result<()>;
}

#[derive(Debug, Default)]
pub(in crate::daemon) struct UnavailableRemoteDesktopRelayLeaseProvider;

impl RemoteDesktopRelayLeaseProvider for UnavailableRemoteDesktopRelayLeaseProvider {
    fn acquire(
        &self,
        _session_id: &str,
        _resource_ura: &str,
    ) -> anyhow::Result<RemoteDesktopRelayLeaseAvailability> {
        Ok(RemoteDesktopRelayLeaseAvailability::unavailable(
            "hub_relay_provider_not_injected",
        ))
    }

    fn release(&self, _lease: &RemoteDesktopRelayLease) -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_init() -> RemoteDesktopRelayLeaseInit {
        RemoteDesktopRelayLeaseInit {
            provider: EASYNET_RELAY_PROVIDER.to_string(),
            lease_id: "lease-1".to_string(),
            session_id: "rd-00112233445566778899aabb".to_string(),
            device_ura: "easynet:///r/acme/device/device-1".to_string(),
            resource_ura: "easynet:///r/acme/resource/device.device-1/streams/window.42"
                .to_string(),
            urls: vec!["turn:relay.example.test:3478?transport=udp".to_string()],
            username: "turn-user".to_string(),
            credential: "turn-secret".to_string(),
            issued_at_ms: 1_000,
            refresh_after_ms: 2_000,
            expires_at_ms: 3_000,
        }
    }

    #[test]
    fn lease_validation_binds_session_and_redacts_debug_and_evidence() {
        let init = valid_init();
        let expected_session_id = init.session_id.clone();
        let expected_resource_ura = init.resource_ura.clone();
        let lease =
            RemoteDesktopRelayLease::from_init(&expected_session_id, &expected_resource_ura, init)
                .expect("valid lease");
        let debug = format!("{lease:?}");
        let evidence = lease.evidence_value().to_string();
        assert!(!debug.contains("turn-secret"));
        assert!(!debug.contains("turn-user"));
        assert!(!evidence.contains("turn-secret"));
        assert!(!evidence.contains("turn-user"));
        assert!(!evidence.contains("credential"));
        assert!(!evidence.contains("username"));
    }

    #[test]
    fn lease_validation_rejects_identity_and_inline_credential_drift() {
        let mut wrong_session = valid_init();
        wrong_session.session_id = "rd-other".to_string();
        assert!(RemoteDesktopRelayLease::from_init(
            "rd-00112233445566778899aabb",
            &wrong_session.resource_ura.clone(),
            wrong_session,
        )
        .is_err());

        let mut inline = valid_init();
        inline.urls = vec!["turn:user:secret@relay.example.test:3478".to_string()];
        assert!(RemoteDesktopRelayLease::from_init(
            &inline.session_id.clone(),
            &inline.resource_ura.clone(),
            inline,
        )
        .is_err());
    }
}
