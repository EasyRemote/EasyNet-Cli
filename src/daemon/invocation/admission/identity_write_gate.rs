// EasyNet CLI — Invocation transport identity write gate
// ======================================================
//
// File: src/daemon/invocation/identity_write_gate.rs
// Description: Daemon-side caller policy for trust-anchor mutations
//              requested through identity write abilities.
//
// Protocol Responsibility:
// This gate does not replace Axon descriptor-bound admission. It runs
// after signature/replay admission and before EasyNet daemon trust
// persistence, deciding whether the admitted caller may author or
// revoke the requested trust row.
//
// Implementation Approach:
// Compare the envelope caller against the current realm trust anchor,
// admission transport boundary, local realm, and target mutation. Keep
// the policy as a small object so register/revoke persistence stays focused
// on validation and atomic writes.
//
// Usage Contract:
// `UnaryDispatcher` must call the matching authorization method before
// invoking a TOML writer. A denied request must not persist or publish
// any trust-anchor change.
//
// Architectural Position:
// EasyNet-Cli daemon owns local runtime/trust policy. EasyNet backend
// may request device/user registrations as a product wrapper, but the
// daemon remains the final authorizer.

use std::sync::Arc;

use easynet_axon::pb::axon::v1::Envelope;
use tonic::Status;

use crate::daemon::invocation::admission::admission_facade::AdmissionTransportBoundary;
use crate::daemon::invocation::admission::register_device_pubkey::RegisterPubkeyIntent;
use crate::daemon::invocation::admission::revoke_user_pubkey::RevokeUserPubkeyIntent;
use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustedAgentRole};

pub(crate) struct IdentityWriteGate {
    trust_anchor: Arc<RealmTrustAnchor>,
    daemon_ura: Option<String>,
    transport_boundary: AdmissionTransportBoundary,
    daemon_realm: String,
}

impl IdentityWriteGate {
    pub(crate) fn new(
        trust_anchor: Arc<RealmTrustAnchor>,
        daemon_ura: Option<String>,
        transport_boundary: AdmissionTransportBoundary,
        daemon_realm: impl Into<String>,
    ) -> Self {
        Self {
            trust_anchor,
            daemon_ura,
            transport_boundary,
            daemon_realm: daemon_realm.into(),
        }
    }

    pub(crate) fn authorize_register_pubkey(
        &self,
        caller_envelope: Option<&Envelope>,
        intent: &RegisterPubkeyIntent,
    ) -> Result<(), Status> {
        let caller = self.authorized_caller(caller_envelope, "identity.register_pubkey")?;
        let caller_ura = caller.ura.as_str();

        match intent.role() {
            TrustedAgentRole::Device => {
                if caller.local_self || self.is_local_backend_or_hub(caller_ura, caller.role) {
                    Ok(())
                } else {
                    Err(self.permission_denied_register(caller_ura, caller.role, intent))
                }
            }
            TrustedAgentRole::User => {
                if caller.local_self || self.is_local_backend_or_hub(caller_ura, caller.role) {
                    return Ok(());
                }
                if caller.role == TrustedAgentRole::Device
                    && self
                        .trust_anchor
                        .lookup_principal_owner(caller_ura)
                        .is_some_and(|owner| owner.owner_ura == intent.agent_ura())
                {
                    return Ok(());
                }
                Err(self.permission_denied_register(caller_ura, caller.role, intent))
            }
            TrustedAgentRole::Backend => {
                if caller.local_self
                    || (self.is_local_backend_or_hub(caller_ura, caller.role)
                        && caller_ura == intent.agent_ura())
                {
                    Ok(())
                } else {
                    Err(self.permission_denied_register(caller_ura, caller.role, intent))
                }
            }
            TrustedAgentRole::Hub => {
                Err(self.permission_denied_register(caller_ura, caller.role, intent))
            }
        }
    }

    pub(crate) fn authorize_revoke_user_pubkey(
        &self,
        caller_envelope: Option<&Envelope>,
        intent: &RevokeUserPubkeyIntent,
    ) -> Result<(), Status> {
        let caller = self.authorized_caller(caller_envelope, "identity.revoke_user_pubkey")?;
        if caller.local_self || self.is_local_backend_or_hub(&caller.ura, caller.role) {
            return Ok(());
        }
        Err(Status::permission_denied(format!(
            "identity.revoke_user_pubkey: caller `{}` with role `{}` cannot revoke user trust row `{}`",
            caller.ura,
            role_label(caller.role),
            intent.agent_ura(),
        )))
    }

    fn authorized_caller(
        &self,
        caller_envelope: Option<&Envelope>,
        ability: &'static str,
    ) -> Result<AuthorizedIdentityWriteCaller, Status> {
        let caller_ura = caller_envelope
            .and_then(|env| env.caller.as_ref())
            .map(|caller| caller.ura.trim())
            .filter(|caller| !caller.is_empty())
            .ok_or_else(|| {
                Status::invalid_argument(format!("{ability}: missing caller envelope.caller.ura"))
            })?;

        if self.is_local_self(caller_ura) {
            return Ok(AuthorizedIdentityWriteCaller {
                ura: caller_ura.to_string(),
                role: TrustedAgentRole::Backend,
                local_self: true,
            });
        }

        let Some(caller_role) = self.trust_anchor.lookup(caller_ura).map(|entry| entry.role) else {
            return Err(Status::permission_denied(format!(
                "{ability}: caller `{caller_ura}` is not trusted to author identity trust rows"
            )));
        };

        Ok(AuthorizedIdentityWriteCaller {
            ura: caller_ura.to_string(),
            role: caller_role,
            local_self: false,
        })
    }

    fn is_local_self(&self, caller_ura: &str) -> bool {
        self.transport_boundary
            .accepts_local_self_caller(self.daemon_ura.as_deref(), caller_ura)
    }

    fn is_local_backend_or_hub(&self, caller_ura: &str, caller_role: TrustedAgentRole) -> bool {
        if !matches!(
            caller_role,
            TrustedAgentRole::Backend | TrustedAgentRole::Hub
        ) {
            return false;
        }
        crate::core::ura::hub_ura(&self.daemon_realm) == caller_ura
    }

    fn permission_denied_register(
        &self,
        caller_ura: &str,
        caller_role: TrustedAgentRole,
        intent: &RegisterPubkeyIntent,
    ) -> Status {
        Status::permission_denied(format!(
            "identity.register_pubkey: caller `{}` with role `{}` cannot author `{}` trust row `{}`",
            caller_ura,
            role_label(caller_role),
            role_label(intent.role()),
            intent.agent_ura(),
        ))
    }
}

#[derive(Debug)]
struct AuthorizedIdentityWriteCaller {
    ura: String,
    role: TrustedAgentRole,
    local_self: bool,
}

fn role_label(role: TrustedAgentRole) -> &'static str {
    match role {
        TrustedAgentRole::Backend => "backend",
        TrustedAgentRole::Device => "device",
        TrustedAgentRole::Hub => "hub",
        TrustedAgentRole::User => "user",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::invocation::admission::revoke_user_pubkey::RevokeUserPubkeyIntent;
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
    use easynet_axon::pb::axon::v1::AgentIdentity;
    use ed25519_dalek::SigningKey;

    fn intent(agent_ura: &str, role: TrustedAgentRole) -> RegisterPubkeyIntent {
        RegisterPubkeyIntent::for_test(agent_ura.to_string(), role)
    }

    fn revoke_intent(agent_ura: &str) -> RevokeUserPubkeyIntent {
        RevokeUserPubkeyIntent::for_test(agent_ura.to_string(), "test-pubkey".to_string())
    }

    fn envelope(caller_ura: &str) -> Envelope {
        Envelope {
            caller: Some(AgentIdentity {
                ura: caller_ura.to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            ..Envelope::default()
        }
    }

    fn anchor_entry(
        agent_ura: &str,
        role: TrustedAgentRole,
    ) -> crate::daemon::trust::anchor::TrustedAgent {
        let key = SigningKey::from_bytes(&[0x42; 32]);
        crate::daemon::trust::anchor::TrustedAgent {
            agent_ura: agent_ura.to_string(),
            public_key_b64: BASE64_STANDARD.encode(key.verifying_key().to_bytes()),
            role,
            added_at_unix_ms: 1_700_000_000_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        }
    }

    fn gate(entries: Vec<crate::daemon::trust::anchor::TrustedAgent>) -> IdentityWriteGate {
        gate_with_owners(entries, Vec::new())
    }

    fn gate_with_owners(
        entries: Vec<crate::daemon::trust::anchor::TrustedAgent>,
        owners: Vec<crate::daemon::trust::anchor::TrustedPrincipalOwner>,
    ) -> IdentityWriteGate {
        IdentityWriteGate::new(
            Arc::new(
                RealmTrustAnchor::from_parts_with_principal_owners(entries, owners, Vec::new())
                    .expect("test trust anchor"),
            ),
            Some("easynet:///r/local/device/daemon".to_string()),
            AdmissionTransportBoundary::LocalOnlyIpc,
            "local",
        )
    }

    #[test]
    fn local_self_can_bootstrap_backend_row_without_anchor_entry() {
        let gate = gate(vec![]);
        let env = envelope("easynet:///r/local/device/daemon");

        gate.authorize_register_pubkey(
            Some(&env),
            &intent(
                &crate::core::ura::hub_ura("local"),
                TrustedAgentRole::Backend,
            ),
        )
        .expect("daemon local self caller owns bootstrap writes");
    }

    #[test]
    fn off_box_boundary_rejects_daemon_ura_spoof_without_anchor_entry() {
        let gate = IdentityWriteGate::new(
            Arc::new(RealmTrustAnchor::default()),
            Some("easynet:///r/local/device/daemon".to_string()),
            AdmissionTransportBoundary::OffBoxStrict,
            "local",
        );
        let env = envelope("easynet:///r/local/device/daemon");

        let err = gate
            .authorize_register_pubkey(
                Some(&env),
                &intent(
                    &crate::core::ura::hub_ura("local"),
                    TrustedAgentRole::Backend,
                ),
            )
            .expect_err("off-box daemon URA spoof must not author trust rows");

        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(
            err.message().contains("not trusted"),
            "unexpected error: {}",
            err.message()
        );
    }

    #[test]
    fn local_backend_can_register_device_and_user_rows() {
        let backend_ura = crate::core::ura::hub_ura("local");
        let gate = gate(vec![anchor_entry(&backend_ura, TrustedAgentRole::Backend)]);
        let env = envelope(&backend_ura);

        gate.authorize_register_pubkey(
            Some(&env),
            &intent(
                "easynet:///r/user-realm/device/dev-1",
                TrustedAgentRole::Device,
            ),
        )
        .expect("backend pairs devices");
        gate.authorize_register_pubkey(
            Some(&env),
            &intent("easynet:///r/local/user/user-1", TrustedAgentRole::User),
        )
        .expect("backend product auth registers user keys");
    }

    #[test]
    fn device_caller_cannot_register_user_row() {
        let device_ura = "easynet:///r/local/device/dev-1";
        let gate = gate(vec![anchor_entry(device_ura, TrustedAgentRole::Device)]);
        let env = envelope(device_ura);

        let err = gate
            .authorize_register_pubkey(
                Some(&env),
                &intent("easynet:///r/local/user/user-1", TrustedAgentRole::User),
            )
            .expect_err("device must not author user trust rows");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(err.message().contains("role `device`"));
    }

    #[test]
    fn paired_device_can_register_only_its_owner_user_row() {
        let device_ura = "easynet:///r/local/device/dev-1";
        let owner_ura = "easynet:///r/local/user/owner-1";
        let gate = gate_with_owners(
            vec![anchor_entry(device_ura, TrustedAgentRole::Device)],
            vec![crate::daemon::trust::anchor::TrustedPrincipalOwner {
                principal_ura: device_ura.to_string(),
                owner_user_id: "owner-1".to_string(),
                owner_ura: owner_ura.to_string(),
                owner_username: None,
                added_at_unix_ms: 1,
            }],
        );
        let env = envelope(device_ura);

        gate.authorize_register_pubkey(Some(&env), &intent(owner_ura, TrustedAgentRole::User))
            .expect("paired device may seed its owner user key");

        let err = gate
            .authorize_register_pubkey(
                Some(&env),
                &intent("easynet:///r/local/user/other", TrustedAgentRole::User),
            )
            .expect_err("paired device must not seed another user's key");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
    }

    #[test]
    fn backend_can_refresh_only_its_own_backend_row() {
        let backend_ura = crate::core::ura::hub_ura("local");
        let gate = gate(vec![anchor_entry(&backend_ura, TrustedAgentRole::Backend)]);
        let env = envelope(&backend_ura);

        gate.authorize_register_pubkey(
            Some(&env),
            &intent(&backend_ura, TrustedAgentRole::Backend),
        )
        .expect("backend may refresh its own row");

        let err = gate
            .authorize_register_pubkey(
                Some(&env),
                &intent(
                    &crate::core::ura::hub_ura("peer"),
                    TrustedAgentRole::Backend,
                ),
            )
            .expect_err("backend must not mint another backend row");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
    }

    #[test]
    fn public_caller_cannot_register_hub_row() {
        let backend_ura = crate::core::ura::hub_ura("local");
        let gate = gate(vec![anchor_entry(&backend_ura, TrustedAgentRole::Backend)]);
        let env = envelope(&backend_ura);

        let err = gate
            .authorize_register_pubkey(
                Some(&env),
                &intent(&crate::core::ura::hub_ura("peer"), TrustedAgentRole::Hub),
            )
            .expect_err("hub trust rows are daemon/operator local");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
    }

    #[test]
    fn local_backend_can_revoke_user_rows() {
        let backend_ura = crate::core::ura::hub_ura("local");
        let gate = gate(vec![anchor_entry(&backend_ura, TrustedAgentRole::Backend)]);
        let env = envelope(&backend_ura);

        gate.authorize_revoke_user_pubkey(
            Some(&env),
            &revoke_intent("easynet:///r/local/user/user-1"),
        )
        .expect("backend product auth revokes user keys");
    }

    #[test]
    fn device_caller_cannot_revoke_user_row() {
        let device_ura = "easynet:///r/local/device/dev-1";
        let gate = gate(vec![anchor_entry(device_ura, TrustedAgentRole::Device)]);
        let env = envelope(device_ura);

        let err = gate
            .authorize_revoke_user_pubkey(
                Some(&env),
                &revoke_intent("easynet:///r/local/user/user-1"),
            )
            .expect_err("device must not revoke user trust rows");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(err.message().contains("role `device`"));
    }
}
