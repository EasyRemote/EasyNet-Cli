// EasyNet CLI — user-binding issue state machine
// ===============================================
//
// File: src/daemon/keyring/user_binding_issue.rs
// Description: Lifecycle for issuing signed federated user-binding tokens.
//
// Protocol Responsibility
// -----------------------
// Execute the deterministic issuer sequence for a `UserBindingToken`: validate
// request realm, load managed-signing authority, validate binding, derive source
// realm, generate nonce, build canonical bytes, and stamp the provider signature.
//
// Implementation Approach
// -----------------------
// `UserBindingIssueStateMachine` owns the ordered issuance transitions and
// returns a signed token. The ability handler only projects JSON arguments into
// a request and serializes the response DTO.
//
// Usage Contract
// --------------
// Callers provide a managed-signing provider, source user URA, managed key id,
// target realm, and issuance timestamp. The state machine preserves existing
// user-facing error strings for invalid authority and realm conditions.
//
// Architectural Position
// ----------------------
// Keyring runtime state-machine layer. Depends on the managed-signing provider
// boundary and token domain, but not on ability registration or response
// serialization.

use anyhow::{anyhow, Result};

use super::managed_signing_provider::ManagedSigningProvider;
use super::user_binding_chain::{
    canonical_user_binding_bytes, UserBindingToken, ED25519_PUBKEY_LEN, USER_BINDING_NONCE_LEN,
};
use super::ManagedSigningStatus;
use crate::core::ura::user_realm_from_ura;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserBindingIssueRequest {
    pub source_user_ura: String,
    pub managed_key_id: String,
    pub target_realm: String,
    pub issued_at_unix_ms: u64,
}

impl UserBindingIssueRequest {
    pub fn new(
        source_user_ura: impl Into<String>,
        managed_key_id: impl Into<String>,
        target_realm: impl Into<String>,
        issued_at_unix_ms: u64,
    ) -> Result<Self> {
        let target_realm = target_realm.into();
        if target_realm.is_empty() {
            return Err(anyhow!("target_realm must be non-empty"));
        }
        Ok(Self {
            source_user_ura: source_user_ura.into(),
            managed_key_id: managed_key_id.into(),
            target_realm,
            issued_at_unix_ms,
        })
    }
}

pub struct UserBindingIssueStateMachine<'a> {
    provider: &'a dyn ManagedSigningProvider,
    request: UserBindingIssueRequest,
}

impl<'a> UserBindingIssueStateMachine<'a> {
    pub fn new(provider: &'a dyn ManagedSigningProvider, request: UserBindingIssueRequest) -> Self {
        Self { provider, request }
    }

    pub fn execute(self) -> Result<UserBindingToken> {
        let signing_entry = self.provider.public_key(&self.request.managed_key_id)?;
        self.ensure_signing_authority(&signing_entry)?;
        let source_realm = self.source_realm()?;
        self.ensure_cross_realm_target(&source_realm)?;
        let source_user_pubkey = decode_source_user_pubkey(&signing_entry.public_key_b64)?;
        let nonce = generate_nonce();
        let mut token = UserBindingToken::new_unsigned(
            source_realm.to_string(),
            self.request.source_user_ura.clone(),
            source_user_pubkey,
            self.request.target_realm.clone(),
            self.request.issued_at_unix_ms,
            nonce,
        );
        let canonical = canonical_user_binding_bytes(&token);
        let sig = self.provider.sign(&signing_entry.key_id, &canonical)?;
        token.signature = sig.to_bytes().to_vec();
        Ok(token)
    }

    fn ensure_signing_authority(
        &self,
        signing_entry: &super::ManagedSigningKeyProjection,
    ) -> Result<()> {
        if signing_entry.status != ManagedSigningStatus::Active
            || signing_entry.purpose != "agent_signing"
            || signing_entry.bound_subject.as_deref() != Some(self.request.source_user_ura.as_str())
        {
            return Err(anyhow!(
                "managed signing authority does not bind active agent_signing key to source_user_ura"
            ));
        }
        Ok(())
    }

    fn source_realm(&self) -> Result<String> {
        user_realm_from_ura(&self.request.source_user_ura).ok_or_else(|| {
            anyhow!(
                "device-subject {:?} is not a canonical \
                 easynet:///r/<realm>/user/<id> URA",
                self.request.source_user_ura
            )
        })
    }

    fn ensure_cross_realm_target(&self, source_realm: &str) -> Result<()> {
        if source_realm == self.request.target_realm {
            return Err(anyhow!(
                "target_realm equals source_realm (`{source_realm}`); \
                 a token issued for the daemon's own realm has no federated meaning"
            ));
        }
        Ok(())
    }
}

fn decode_source_user_pubkey(public_key_b64: &str) -> Result<[u8; ED25519_PUBKEY_LEN]> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    let pubkey_raw = STANDARD
        .decode(public_key_b64)
        .map_err(|e| anyhow!("base64 decode: {e}"))?;
    pubkey_raw.as_slice().try_into().map_err(|_| {
        anyhow!(
            "agent_signing entry has wrong-length pubkey: {} bytes (expected {})",
            pubkey_raw.len(),
            ED25519_PUBKEY_LEN
        )
    })
}

fn generate_nonce() -> [u8; USER_BINDING_NONCE_LEN] {
    let mut nonce = [0u8; USER_BINDING_NONCE_LEN];
    use rand::RngCore;
    rand::thread_rng().fill_bytes(&mut nonce);
    nonce
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::keyring::{ManagedPeer, ManagedSigningKeyProjection, ED25519_SEED_LEN};
    use ed25519_dalek::SigningKey;

    struct TestProvider {
        key: SigningKey,
        projection: ManagedSigningKeyProjection,
    }

    impl TestProvider {
        fn active_bound() -> Self {
            let key = SigningKey::from_bytes(&[0x52; ED25519_SEED_LEN]);
            Self {
                projection: ManagedSigningKeyProjection {
                    key_id: "key-1".to_string(),
                    purpose: "agent_signing".to_string(),
                    public_key_b64: encode_b64(&key.verifying_key().to_bytes()),
                    status: ManagedSigningStatus::Active,
                    rotation_epoch: 0,
                    bound_subject: Some("easynet:///r/realm-a/user/user-c".to_string()),
                    signer_policy_ref: None,
                    rotated_from: None,
                    created_unix_ms: 1,
                    expires_unix_ms: None,
                    revoked_unix_ms: None,
                },
                key,
            }
        }
    }

    impl ManagedSigningProvider for TestProvider {
        fn create(
            &self,
            _purpose: String,
            _bound_subject: Option<String>,
        ) -> Result<ManagedSigningKeyProjection> {
            unreachable!("issuer state machine does not create keys")
        }

        fn list(
            &self,
            _purpose: Option<String>,
            _status: Option<ManagedSigningStatus>,
        ) -> Result<Vec<ManagedSigningKeyProjection>> {
            unreachable!("issuer state machine does not list keys")
        }

        fn public_key(&self, _key_id: &str) -> Result<ManagedSigningKeyProjection> {
            Ok(self.projection.clone())
        }

        fn sign(&self, _key_id: &str, canonical_bytes: &[u8]) -> Result<ed25519_dalek::Signature> {
            use ed25519_dalek::Signer;
            Ok(self.key.sign(canonical_bytes))
        }

        fn rotate(&self, _key_id: &str) -> Result<ManagedSigningKeyProjection> {
            unreachable!("issuer state machine does not rotate keys")
        }

        fn revoke(&self, _key_id: &str) -> Result<i64> {
            unreachable!("issuer state machine does not revoke keys")
        }

        fn set_expiry(&self, _key_id: &str, _expires_unix_ms: i64) -> Result<()> {
            unreachable!("issuer state machine does not set expiry")
        }

        fn bind_subject(&self, _key_id: &str, _subject_ura: &str) -> Result<()> {
            unreachable!("issuer state machine does not bind subjects")
        }

        fn peer_add(
            &self,
            _peer_ura: &str,
            _public_key_b64: &str,
            _via_hub: Option<String>,
        ) -> Result<bool> {
            unreachable!("issuer state machine does not add peers")
        }

        fn peer_list(&self) -> Result<Vec<ManagedPeer>> {
            unreachable!("issuer state machine does not list peers")
        }
    }

    fn request(target_realm: &str) -> UserBindingIssueRequest {
        UserBindingIssueRequest::new(
            "easynet:///r/realm-a/user/user-c",
            "key-1",
            target_realm,
            1_714_500_000_000,
        )
        .expect("valid request")
    }

    #[test]
    fn issue_state_machine_returns_signed_token() {
        let provider = TestProvider::active_bound();
        let token = UserBindingIssueStateMachine::new(&provider, request("realm-b"))
            .execute()
            .expect("issue succeeds");

        assert_eq!(token.source_realm, "realm-a");
        assert_eq!(token.source_user_ura, "easynet:///r/realm-a/user/user-c");
        assert_eq!(token.target_realm, "realm-b");
        assert_eq!(token.issued_at_ms, 1_714_500_000_000);
        assert_eq!(token.source_user_pubkey.len(), ED25519_PUBKEY_LEN);
        assert_eq!(token.nonce.len(), USER_BINDING_NONCE_LEN);
        assert_eq!(token.signature.len(), 64);
        super::super::user_binding_chain::verify_user_binding_signature(&token)
            .expect("issued token signature verifies");
    }

    #[test]
    fn issue_state_machine_rejects_self_target_realm() {
        let provider = TestProvider::active_bound();
        let error = UserBindingIssueStateMachine::new(&provider, request("realm-a"))
            .execute()
            .expect_err("self-target rejects");

        assert!(error.to_string().contains("source_realm"));
    }

    #[test]
    fn issue_state_machine_rejects_unbound_signing_key() {
        let mut provider = TestProvider::active_bound();
        provider.projection.bound_subject = None;

        let error = UserBindingIssueStateMachine::new(&provider, request("realm-b"))
            .execute()
            .expect_err("unbound signing key rejects");

        assert!(error.to_string().contains("does not bind"));
    }

    fn encode_b64(bytes: &[u8]) -> String {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;
        STANDARD.encode(bytes)
    }
}
