// EasyNet CLI — remote desktop consent registry
// ==============================================
//
// File: plugins/remote-desktop/src/consent_registry.rs
// Description: Bounded one-use local consent capabilities.
//
// Protocol Responsibility:
// - None. Axon verifies Invocation and receipt authenticity; this registry owns
//   remote-desktop product authorization semantics.
//
// Implementation Approach:
// - Mint an unguessable short-lived ticket bound to caller, resource, and
//   intent; consume it exactly once during session creation.
//
// Usage Contract:
// - A causal receipt is an audit parent, not a substitute for this capability.
//
// Architectural Position:
// - Plugin runtime security component.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use rand::RngCore as _;
use sha2::{Digest as _, Sha256};

use crate::daemon::plugins::remote_desktop::session::now_ms;

const CONSENT_TICKET_TTL_MS: u64 = 60_000;
pub(in crate::daemon::plugins::remote_desktop) const CONSENT_INTENT: &str =
    "remote_desktop_session";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopConsentAuthorization {
    pub(in crate::daemon::plugins::remote_desktop) consent_id: String,
    pub(in crate::daemon::plugins::remote_desktop) caller_ura: String,
    pub(in crate::daemon::plugins::remote_desktop) subject_ura: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct IssuedConsentTicket {
    pub(in crate::daemon::plugins::remote_desktop) ticket: String,
    pub(in crate::daemon::plugins::remote_desktop) expires_at_ms: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) enum ConsentTicketError {
    #[error("remote desktop consent registry is full")]
    Full,
    #[error("remote desktop consent ticket is missing, expired, or already consumed")]
    Invalid,
    #[error("remote desktop consent ticket caller does not match")]
    CallerMismatch,
    #[error("remote desktop consent ticket resource does not match")]
    SubjectMismatch,
    #[error("remote desktop consent ticket intent does not match")]
    IntentMismatch,
}

#[derive(Debug, Clone)]
struct PendingConsent {
    caller_ura: String,
    subject_ura: String,
    intent: String,
    expires_at_ms: u64,
}

#[derive(Debug)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopConsentRegistry {
    pending: Mutex<HashMap<String, PendingConsent>>,
    max_pending: usize,
}

impl RemoteDesktopConsentRegistry {
    pub(in crate::daemon::plugins::remote_desktop) fn new(max_pending: usize) -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            max_pending: max_pending.max(1),
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn issue(
        &self,
        caller_ura: &str,
        subject_ura: &str,
        intent: &str,
    ) -> Result<IssuedConsentTicket, ConsentTicketError> {
        let now = now_ms();
        let mut pending = self.pending();
        pending.retain(|_, grant| grant.expires_at_ms > now);
        if pending.len() >= self.max_pending {
            return Err(ConsentTicketError::Full);
        }
        let ticket = mint_ticket();
        let expires_at_ms = now.saturating_add(CONSENT_TICKET_TTL_MS);
        pending.insert(
            ticket.clone(),
            PendingConsent {
                caller_ura: caller_ura.to_string(),
                subject_ura: subject_ura.to_string(),
                intent: intent.to_string(),
                expires_at_ms,
            },
        );
        Ok(IssuedConsentTicket {
            ticket,
            expires_at_ms,
        })
    }

    pub(in crate::daemon::plugins::remote_desktop) fn consume(
        &self,
        ticket: &str,
        caller_ura: &str,
        subject_ura: &str,
        intent: &str,
    ) -> Result<RemoteDesktopConsentAuthorization, ConsentTicketError> {
        let grant = self
            .pending()
            .remove(ticket)
            .filter(|grant| grant.expires_at_ms > now_ms())
            .ok_or(ConsentTicketError::Invalid)?;
        if grant.intent != intent {
            return Err(ConsentTicketError::IntentMismatch);
        }
        if grant.caller_ura != caller_ura {
            return Err(ConsentTicketError::CallerMismatch);
        }
        if grant.subject_ura != subject_ura {
            return Err(ConsentTicketError::SubjectMismatch);
        }
        Ok(RemoteDesktopConsentAuthorization {
            consent_id: hex::encode(Sha256::digest(ticket.as_bytes())),
            caller_ura: grant.caller_ura,
            subject_ura: grant.subject_ura,
        })
    }

    fn pending(&self) -> MutexGuard<'_, HashMap<String, PendingConsent>> {
        match self.pending.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

fn mint_ticket() -> String {
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consent_ticket_is_bound_and_one_use() {
        let registry = RemoteDesktopConsentRegistry::new(2);
        let issued = registry
            .issue("caller", "resource", CONSENT_INTENT)
            .unwrap();
        assert_eq!(
            registry.consume(&issued.ticket, "other", "resource", CONSENT_INTENT),
            Err(ConsentTicketError::CallerMismatch)
        );
        assert_eq!(
            registry.consume(&issued.ticket, "caller", "resource", CONSENT_INTENT),
            Err(ConsentTicketError::Invalid),
            "a mismatched attempt consumes the capability and fails closed"
        );
    }

    #[test]
    fn consent_registry_is_hard_bounded() {
        let registry = RemoteDesktopConsentRegistry::new(1);
        registry
            .issue("caller", "resource", CONSENT_INTENT)
            .unwrap();
        assert_eq!(
            registry.issue("caller", "resource", CONSENT_INTENT),
            Err(ConsentTicketError::Full)
        );
    }
}
