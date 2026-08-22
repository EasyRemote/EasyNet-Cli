// EasyNet CLI — remote desktop consent state machine
// ==================================================
//
// File: plugins/remote-desktop/src/session_consent_state.rs
// Description: Session-owned EasyNet consent lifecycle for one targeted session.

use serde_json::{json, Value};

use crate::daemon::plugins::remote_desktop::session_consent::RemoteDesktopConsentGrant;

/// Canonical consent lifecycle for an inserted remote desktop session.
///
/// `NotRequested`, `Requested`, and `Granted` are creation-workflow states in
/// the SPEC. Inserted session aggregates start at `Active` because
/// `create_session` has already consumed a scoped consent ticket and causal
/// receipt before constructing the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) enum RemoteDesktopConsentPhase {
    Active,
    Revoked,
    Expired,
}

impl RemoteDesktopConsentPhase {
    pub(in crate::daemon::plugins::remote_desktop) const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Revoked => "revoked",
            Self::Expired => "expired",
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) const fn permits_media_input(self) -> bool {
        matches!(self, Self::Active)
    }
}

/// Aggregate-owned consent state.
///
/// The immutable grant remains the audit fact; this state machine owns whether
/// that grant is currently allowed to drive media and input for the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopConsentState {
    grant: RemoteDesktopConsentGrant,
    phase: RemoteDesktopConsentPhase,
    consent_epoch: u64,
}

impl RemoteDesktopConsentState {
    pub(in crate::daemon::plugins::remote_desktop) fn active(
        grant: RemoteDesktopConsentGrant,
        consent_epoch: u64,
    ) -> Self {
        Self {
            grant,
            phase: RemoteDesktopConsentPhase::Active,
            consent_epoch,
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn rehydrate(
        grant: RemoteDesktopConsentGrant,
        value: &Value,
    ) -> anyhow::Result<Self> {
        let phase = match value
            .get("phase")
            .and_then(Value::as_str)
            .unwrap_or("active")
        {
            "active" => RemoteDesktopConsentPhase::Active,
            "revoked" => RemoteDesktopConsentPhase::Revoked,
            "expired" => RemoteDesktopConsentPhase::Expired,
            other => anyhow::bail!("unsupported RemoteApp recovery consent phase {other:?}"),
        };
        let consent_epoch = value
            .get("consent_epoch")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow::anyhow!("RemoteApp recovery consent requires consent_epoch"))?;
        Ok(Self {
            grant,
            phase,
            consent_epoch,
        })
    }

    pub(in crate::daemon::plugins::remote_desktop) const fn phase(
        &self,
    ) -> RemoteDesktopConsentPhase {
        self.phase
    }

    pub(in crate::daemon::plugins::remote_desktop) const fn permits_media_input(&self) -> bool {
        self.phase.permits_media_input()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn grant(&self) -> &RemoteDesktopConsentGrant {
        &self.grant
    }

    pub(in crate::daemon::plugins::remote_desktop) fn revoke(&mut self) -> bool {
        self.transition_terminal(RemoteDesktopConsentPhase::Revoked)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn expire(&mut self) -> bool {
        self.transition_terminal(RemoteDesktopConsentPhase::Expired)
    }

    fn transition_terminal(&mut self, phase: RemoteDesktopConsentPhase) -> bool {
        if self.phase != RemoteDesktopConsentPhase::Active {
            return false;
        }
        self.phase = phase;
        self.consent_epoch = self.consent_epoch.saturating_add(1);
        true
    }

    pub(in crate::daemon::plugins::remote_desktop) fn to_value(&self) -> Value {
        let mut value = self.grant.to_value();
        if let Some(map) = value.as_object_mut() {
            map.insert("phase".to_string(), json!(self.phase.as_str()));
            map.insert("active".to_string(), json!(self.permits_media_input()));
            map.insert("consent_epoch".to_string(), json!(self.consent_epoch));
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::plugins::remote_desktop::test_support::env_for;

    #[test]
    fn inserted_session_consent_starts_active_and_terminal_transitions_bump_epoch() {
        let env = env_for("easynet:///r/acme/resource/display.01");
        let grant = RemoteDesktopConsentGrant::from_envelope_for_test(&env);
        let mut state = RemoteDesktopConsentState::active(grant, 3);

        assert_eq!(state.phase(), RemoteDesktopConsentPhase::Active);
        assert!(state.permits_media_input());
        assert_eq!(state.to_value()["consent_epoch"], json!(3));

        assert!(state.revoke());
        assert_eq!(state.phase(), RemoteDesktopConsentPhase::Revoked);
        assert!(!state.permits_media_input());
        assert_eq!(state.to_value()["consent_epoch"], json!(4));
        assert!(!state.expire());
        assert_eq!(state.to_value()["consent_epoch"], json!(4));
    }
}
