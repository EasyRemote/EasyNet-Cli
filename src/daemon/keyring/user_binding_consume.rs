// EasyNet CLI — user-binding consume state machine
// =================================================
//
// File: src/daemon/keyring/user_binding_consume.rs
// Description: Admission/replay/write lifecycle for consuming user-binding tokens.
//
// Protocol Responsibility
// -----------------------
// Execute the deterministic consume sequence for a federated
// `UserBindingToken`: target realm, freshness, signature, replay, and binding
// persistence. This is the authority path for token consumption.
//
// Implementation Approach
// -----------------------
// `UserBindingConsumeStateMachine` owns the ordered transitions and returns the
// persisted public binding projection on success. The ability handler only
// projects JSON arguments into a request and serializes the response DTO.
//
// Usage Contract
// --------------
// Callers must provide an already-decoded token, the consuming realm, the local
// authenticated user id, and the current Unix-millisecond clock value. The
// state machine preserves the existing user-facing error strings for each
// rejection stage.
//
// Architectural Position
// ----------------------
// Keyring runtime state-machine layer. Depends on token verification and the
// federated binding store, but not on ability registration, managed-signing
// provider custody, or response serialization.

use anyhow::{anyhow, Result};

use super::federated_bindings::{FederatedBindingsStore, FederatedUserBinding};
use super::user_binding_chain::{
    verify_user_binding_signature, UserBindingError, UserBindingToken, USER_BINDING_FRESHNESS_MS,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserBindingConsumeRequest {
    pub token: UserBindingToken,
    pub self_realm: String,
    pub local_user_id: String,
    pub now_unix_ms: u64,
}

impl UserBindingConsumeRequest {
    pub fn new(
        token: UserBindingToken,
        self_realm: impl Into<String>,
        local_user_id: impl Into<String>,
        now_unix_ms: u64,
    ) -> Result<Self> {
        let self_realm = self_realm.into();
        if self_realm.is_empty() {
            return Err(anyhow!("self_realm must be non-empty"));
        }
        let local_user_id = local_user_id.into();
        if local_user_id.is_empty() {
            return Err(anyhow!("local_user_id must be non-empty"));
        }
        Ok(Self {
            token,
            self_realm,
            local_user_id,
            now_unix_ms,
        })
    }
}

pub struct UserBindingConsumeStateMachine<'a> {
    bindings: &'a FederatedBindingsStore,
    request: UserBindingConsumeRequest,
}

impl<'a> UserBindingConsumeStateMachine<'a> {
    pub fn new(bindings: &'a FederatedBindingsStore, request: UserBindingConsumeRequest) -> Self {
        Self { bindings, request }
    }

    pub fn execute(self) -> Result<FederatedUserBinding> {
        self.ensure_target_realm()?;
        self.ensure_freshness()?;
        self.ensure_signature()?;
        let nonce_b64 = self.nonce_b64();
        self.ensure_not_replayed(&nonce_b64)?;
        self.record_binding(nonce_b64)
    }

    fn ensure_target_realm(&self) -> Result<()> {
        if self.request.token.target_realm != self.request.self_realm {
            let err = UserBindingError::WrongTargetRealm {
                expected: self.request.self_realm.clone(),
                actual: self.request.token.target_realm.clone(),
            };
            return Err(anyhow!("{}", err));
        }
        Ok(())
    }

    fn ensure_freshness(&self) -> Result<()> {
        if self
            .request
            .now_unix_ms
            .saturating_sub(self.request.token.issued_at_ms)
            > USER_BINDING_FRESHNESS_MS
        {
            let err = UserBindingError::ExpiredToken {
                issued_at_ms: self.request.token.issued_at_ms,
                now_ms: self.request.now_unix_ms,
            };
            return Err(anyhow!("{}", err));
        }
        if self.request.token.issued_at_ms
            > self
                .request
                .now_unix_ms
                .saturating_add(USER_BINDING_FRESHNESS_MS)
        {
            let err = UserBindingError::ExpiredToken {
                issued_at_ms: self.request.token.issued_at_ms,
                now_ms: self.request.now_unix_ms,
            };
            return Err(anyhow!("future-dated token: {}", err));
        }
        Ok(())
    }

    fn ensure_signature(&self) -> Result<()> {
        verify_user_binding_signature(&self.request.token).map_err(|err| anyhow!("{}", err))
    }

    fn ensure_not_replayed(&self, nonce_b64: &str) -> Result<()> {
        if self
            .bindings
            .nonce_seen(&self.request.token.source_realm, nonce_b64)
        {
            return Err(anyhow!("{}", UserBindingError::ReplayDetected));
        }
        Ok(())
    }

    fn record_binding(self, nonce_b64: String) -> Result<FederatedUserBinding> {
        let binding = FederatedUserBinding {
            source_realm: self.request.token.source_realm.clone(),
            source_user_ura: self.request.token.source_user_ura.clone(),
            source_user_pubkey_b64: encode_b64(&self.request.token.source_user_pubkey),
            local_user_id: self.request.local_user_id.clone(),
            bound_at_unix_ms: i64::try_from(self.request.now_unix_ms).unwrap_or(i64::MAX),
        };
        self.bindings.record_binding(binding.clone(), nonce_b64)?;
        Ok(binding)
    }

    fn nonce_b64(&self) -> String {
        encode_b64(&self.request.token.nonce)
    }
}

fn encode_b64(bytes: &[u8]) -> String {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::keyring::user_binding_chain::{
        sign_user_binding_token, ED25519_PUBKEY_LEN, USER_BINDING_NONCE_LEN,
    };
    use ed25519_dalek::SigningKey;

    fn signed_token(target_realm: &str, issued_at_ms: u64) -> UserBindingToken {
        let signing = SigningKey::from_bytes(&[0x42; 32]);
        let mut token = UserBindingToken::new_unsigned(
            "realm-a",
            "easynet:///r/realm-a/user/user-c",
            signing.verifying_key().to_bytes(),
            target_realm,
            issued_at_ms,
            [0xAA; USER_BINDING_NONCE_LEN],
        );
        sign_user_binding_token(&mut token, &signing);
        token
    }

    fn request(token: UserBindingToken) -> UserBindingConsumeRequest {
        UserBindingConsumeRequest::new(
            token,
            "realm-b",
            "user-c-on-realm-b",
            1_714_500_000_000_u64 + 1_000,
        )
        .expect("valid request")
    }

    #[test]
    fn consume_state_machine_records_binding_after_all_checks() {
        let bindings = FederatedBindingsStore::in_memory();
        let binding = UserBindingConsumeStateMachine::new(
            &bindings,
            request(signed_token("realm-b", 1_714_500_000_000)),
        )
        .execute()
        .expect("consume succeeds");

        assert_eq!(binding.source_realm, "realm-a");
        assert_eq!(binding.source_user_ura, "easynet:///r/realm-a/user/user-c");
        assert_eq!(binding.local_user_id, "user-c-on-realm-b");
        assert_eq!(
            base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                &binding.source_user_pubkey_b64,
            )
            .expect("public key decodes")
            .len(),
            ED25519_PUBKEY_LEN
        );
        assert_eq!(
            bindings
                .find_local_user("realm-a", "easynet:///r/realm-a/user/user-c")
                .as_deref(),
            Some("user-c-on-realm-b")
        );
    }

    #[test]
    fn consume_state_machine_rejects_wrong_target_before_recording() {
        let bindings = FederatedBindingsStore::in_memory();
        let error = UserBindingConsumeStateMachine::new(
            &bindings,
            request(signed_token("realm-c", 1_714_500_000_000)),
        )
        .execute()
        .expect_err("wrong target must reject");

        assert!(error.to_string().contains("wrong target_realm"));
        assert!(bindings.list().is_empty());
    }

    #[test]
    fn consume_state_machine_rejects_replay_before_second_recording() {
        let bindings = FederatedBindingsStore::in_memory();
        let token = signed_token("realm-b", 1_714_500_000_000);
        UserBindingConsumeStateMachine::new(&bindings, request(token.clone()))
            .execute()
            .expect("first consume succeeds");

        let error = UserBindingConsumeStateMachine::new(&bindings, request(token))
            .execute()
            .expect_err("second consume rejects");

        assert!(error.to_string().contains("replay detected"));
        assert_eq!(bindings.list().len(), 1);
    }
}
