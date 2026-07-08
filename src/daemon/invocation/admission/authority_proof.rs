// EasyNet CLI — RFC-014 AuthorityProof model
// ===========================================

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use chrono::{DateTime, Utc};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::decision::{AccessAction, PrincipalKind};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct AuthorityProof {
    pub proof_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_request_id: Option<String>,
    pub owner_user_id: String,
    pub principal_kind: PrincipalKind,
    pub principal_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_id: Option<String>,
    pub callee_ura: String,
    pub subject_ura: String,
    pub ability_ura: String,
    pub action: AccessAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_owner_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_followup_abilities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_expires_at: Option<String>,
    pub issued_at: String,
    pub expires_at: String,
    pub issuer_ura: String,
    pub audience_ura: String,
    pub signature: String,
}

impl AuthorityProof {
    #[must_use]
    pub fn canonical_material(&self) -> Value {
        let abilities = normalized_followup_abilities(&self.allowed_followup_abilities);
        let mut value = json!({
            "profile": "easynet-authority-proof-v0",
            "proof_id": self.proof_id,
            "owner_user_id": self.owner_user_id,
            "principal_kind": self.principal_kind,
            "principal_id": self.principal_id,
            "callee_ura": self.callee_ura,
            "subject_ura": self.subject_ura,
            "ability_ura": self.ability_ura,
            "action": self.action,
            "issued_at": self.issued_at,
            "expires_at": self.expires_at,
            "issuer_ura": self.issuer_ura,
            "audience_ura": self.audience_ura,
        });
        insert_optional(&mut value, "grant_id", self.grant_id.as_deref());
        insert_optional(
            &mut value,
            "permission_request_id",
            self.permission_request_id.as_deref(),
        );
        insert_optional(&mut value, "token_id", self.token_id.as_deref());
        insert_optional(&mut value, "nonce", self.nonce.as_deref());
        insert_optional(&mut value, "canonical_hash", self.canonical_hash.as_deref());
        insert_optional(&mut value, "session_id", self.session_id.as_deref());
        insert_optional(
            &mut value,
            "session_owner_user_id",
            self.session_owner_user_id.as_deref(),
        );
        if !abilities.is_empty() {
            value["allowed_followup_abilities"] = json!(abilities);
        }
        insert_optional(
            &mut value,
            "session_expires_at",
            self.session_expires_at.as_deref(),
        );
        value
    }

    #[must_use]
    pub fn canonical_hash(&self) -> String {
        let bytes = crate::daemon::ability::canonical_json_bytes(&self.canonical_material());
        format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityProofVerificationContext<'a> {
    pub owner_user_id: &'a str,
    pub principal_kind: PrincipalKind,
    pub principal_id: &'a str,
    pub token_id: Option<&'a str>,
    pub callee_ura: &'a str,
    pub subject_ura: &'a str,
    pub ability_ura: &'a str,
    pub action: AccessAction,
    pub nonce: Option<&'a str>,
    pub canonical_hash: Option<&'a str>,
    pub audience_ura: &'a str,
    pub session_id: Option<&'a str>,
    pub session_owner_user_id: Option<&'a str>,
    pub now: DateTime<Utc>,
}

pub trait AuthorityProofIssuerResolver {
    fn verifying_key_for_issuer(&self, issuer_ura: &str) -> Option<VerifyingKey>;
    fn issuer_authorized_for_owner(&self, issuer_ura: &str, owner_user_id: &str) -> bool;
    fn referenced_authority_active(&self, proof: &AuthorityProof) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AuthorityProofDenyReason {
    AuthorityProofMissing,
    AuthorityProofExpired,
    AuthorityProofSignatureInvalid,
    AuthorityProofIssuerDenied,
    AuthorityProofAudienceMismatch,
    AuthorityProofMismatch,
    AuthorityProofRevoked,
}

pub struct AuthorityProofVerifier;

impl AuthorityProofVerifier {
    pub fn verify(
        proof: Option<&AuthorityProof>,
        context: &AuthorityProofVerificationContext<'_>,
        resolver: &dyn AuthorityProofIssuerResolver,
    ) -> Result<(), AuthorityProofDenyReason> {
        let proof = proof.ok_or(AuthorityProofDenyReason::AuthorityProofMissing)?;
        verify_not_expired(proof, context.now)?;
        verify_invocation_binding(proof, context)?;
        verify_signature(proof, resolver)?;
        if !resolver.issuer_authorized_for_owner(&proof.issuer_ura, &proof.owner_user_id) {
            return Err(AuthorityProofDenyReason::AuthorityProofIssuerDenied);
        }
        if !resolver.referenced_authority_active(proof) {
            return Err(AuthorityProofDenyReason::AuthorityProofRevoked);
        }
        Ok(())
    }
}

fn verify_not_expired(
    proof: &AuthorityProof,
    now: DateTime<Utc>,
) -> Result<(), AuthorityProofDenyReason> {
    let expires_at = DateTime::parse_from_rfc3339(&proof.expires_at)
        .map_err(|_| AuthorityProofDenyReason::AuthorityProofMismatch)?
        .with_timezone(&Utc);
    if expires_at <= now {
        return Err(AuthorityProofDenyReason::AuthorityProofExpired);
    }
    if let Some(session_expires_at) = proof.session_expires_at.as_deref() {
        let session_expires_at = DateTime::parse_from_rfc3339(session_expires_at)
            .map_err(|_| AuthorityProofDenyReason::AuthorityProofMismatch)?
            .with_timezone(&Utc);
        if session_expires_at <= now {
            return Err(AuthorityProofDenyReason::AuthorityProofExpired);
        }
    }
    Ok(())
}

fn verify_invocation_binding(
    proof: &AuthorityProof,
    context: &AuthorityProofVerificationContext<'_>,
) -> Result<(), AuthorityProofDenyReason> {
    if proof.audience_ura != context.audience_ura {
        return Err(AuthorityProofDenyReason::AuthorityProofAudienceMismatch);
    }
    if proof.owner_user_id != context.owner_user_id
        || proof.principal_kind != context.principal_kind
        || proof.principal_id != context.principal_id
        || proof.token_id.as_deref() != context.token_id
        || proof.callee_ura != context.callee_ura
        || proof.subject_ura != context.subject_ura
        || proof.ability_ura != context.ability_ura
        || proof.action != context.action
    {
        return Err(AuthorityProofDenyReason::AuthorityProofMismatch);
    }
    if proof.nonce.as_deref().is_some() && proof.nonce.as_deref() != context.nonce {
        return Err(AuthorityProofDenyReason::AuthorityProofMismatch);
    }
    if proof.canonical_hash.as_deref().is_some()
        && proof.canonical_hash.as_deref() != context.canonical_hash
    {
        return Err(AuthorityProofDenyReason::AuthorityProofMismatch);
    }
    if proof.session_id.as_deref().is_some() && proof.session_id.as_deref() != context.session_id {
        return Err(AuthorityProofDenyReason::AuthorityProofMismatch);
    }
    if proof.session_owner_user_id.as_deref().is_some()
        && proof.session_owner_user_id.as_deref() != context.session_owner_user_id
    {
        return Err(AuthorityProofDenyReason::AuthorityProofMismatch);
    }
    if proof.session_id.is_some() {
        let allowed = normalized_followup_abilities(&proof.allowed_followup_abilities);
        if allowed.is_empty() {
            return Err(AuthorityProofDenyReason::AuthorityProofMismatch);
        }
        if !allowed.iter().any(|ability| ability == context.ability_ura) {
            return Err(AuthorityProofDenyReason::AuthorityProofMismatch);
        }
    }
    Ok(())
}

fn normalized_followup_abilities(abilities: &[String]) -> Vec<String> {
    let mut abilities = abilities
        .iter()
        .map(|ability| ability.trim().to_string())
        .filter(|ability| !ability.is_empty())
        .collect::<Vec<_>>();
    abilities.sort();
    abilities.dedup();
    abilities
}

fn verify_signature(
    proof: &AuthorityProof,
    resolver: &dyn AuthorityProofIssuerResolver,
) -> Result<(), AuthorityProofDenyReason> {
    let key = resolver
        .verifying_key_for_issuer(&proof.issuer_ura)
        .ok_or(AuthorityProofDenyReason::AuthorityProofIssuerDenied)?;
    let signature = decode_signature(&proof.signature)?;
    let bytes = crate::daemon::ability::canonical_json_bytes(&proof.canonical_material());
    key.verify(&bytes, &signature)
        .map_err(|_| AuthorityProofDenyReason::AuthorityProofSignatureInvalid)
}

fn decode_signature(raw: &str) -> Result<Signature, AuthorityProofDenyReason> {
    let raw = raw.trim();
    let encoded = raw.strip_prefix("ed25519:").unwrap_or(raw);
    let bytes = BASE64_STANDARD
        .decode(encoded)
        .map_err(|_| AuthorityProofDenyReason::AuthorityProofSignatureInvalid)?;
    let bytes: [u8; 64] = bytes
        .try_into()
        .map_err(|_| AuthorityProofDenyReason::AuthorityProofSignatureInvalid)?;
    Ok(Signature::from_bytes(&bytes))
}

fn insert_optional(value: &mut Value, key: &str, candidate: Option<&str>) {
    if let Some(candidate) = candidate.filter(|raw| !raw.trim().is_empty()) {
        value[key] = json!(candidate);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use std::collections::BTreeMap;

    #[test]
    fn canonical_material_sorts_and_deduplicates_followup_abilities() {
        let proof = AuthorityProof {
            proof_id: "proof-1".to_string(),
            grant_id: Some("grant-1".to_string()),
            permission_request_id: None,
            owner_user_id: "alice".to_string(),
            principal_kind: PrincipalKind::Token,
            principal_id: "token-principal".to_string(),
            token_id: Some("token-1".to_string()),
            callee_ura: "easynet:///r/test/device/dev".to_string(),
            subject_ura: "easynet:///r/test/session/s1".to_string(),
            ability_ura: "terminal.attach".to_string(),
            action: AccessAction::Stream,
            nonce: None,
            canonical_hash: Some("sha256:abc".to_string()),
            session_id: Some("s1".to_string()),
            session_owner_user_id: Some("alice".to_string()),
            allowed_followup_abilities: vec![
                "terminal.read".to_string(),
                "terminal.attach".to_string(),
                "terminal.read".to_string(),
            ],
            session_expires_at: Some("2026-07-09T01:00:00Z".to_string()),
            issued_at: "2026-07-09T00:00:00Z".to_string(),
            expires_at: "2026-07-09T00:05:00Z".to_string(),
            issuer_ura: "easynet:///r/test/user/alice".to_string(),
            audience_ura: "easynet:///r/test/device/dev".to_string(),
            signature: "sig".to_string(),
        };
        assert_eq!(
            proof.canonical_material()["allowed_followup_abilities"],
            json!(["terminal.attach", "terminal.read"])
        );
    }

    struct TestIssuerResolver {
        keys: BTreeMap<String, VerifyingKey>,
        authorized: bool,
        active: bool,
    }

    impl AuthorityProofIssuerResolver for TestIssuerResolver {
        fn verifying_key_for_issuer(&self, issuer_ura: &str) -> Option<VerifyingKey> {
            self.keys.get(issuer_ura).copied()
        }

        fn issuer_authorized_for_owner(&self, _issuer_ura: &str, _owner_user_id: &str) -> bool {
            self.authorized
        }

        fn referenced_authority_active(&self, _proof: &AuthorityProof) -> bool {
            self.active
        }
    }

    fn signed_proof() -> (AuthorityProof, TestIssuerResolver) {
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let issuer = "easynet:///r/test/user/alice".to_string();
        let mut proof = AuthorityProof {
            proof_id: "proof-1".to_string(),
            grant_id: Some("grant-1".to_string()),
            permission_request_id: None,
            owner_user_id: "alice".to_string(),
            principal_kind: PrincipalKind::Token,
            principal_id: "token-principal".to_string(),
            token_id: Some("token-1".to_string()),
            callee_ura: "easynet:///r/test/device/dev".to_string(),
            subject_ura: "easynet:///r/test/session/s1".to_string(),
            ability_ura: "terminal.attach".to_string(),
            action: AccessAction::Stream,
            nonce: None,
            canonical_hash: Some("sha256:abc".to_string()),
            session_id: Some("s1".to_string()),
            session_owner_user_id: Some("alice".to_string()),
            allowed_followup_abilities: vec!["terminal.attach".to_string()],
            session_expires_at: Some("2026-07-09T01:00:00Z".to_string()),
            issued_at: "2026-07-09T00:00:00Z".to_string(),
            expires_at: "2026-07-09T00:05:00Z".to_string(),
            issuer_ura: issuer.clone(),
            audience_ura: "easynet:///r/test/device/dev".to_string(),
            signature: String::new(),
        };
        let bytes = crate::daemon::ability::canonical_json_bytes(&proof.canonical_material());
        proof.signature = format!(
            "ed25519:{}",
            BASE64_STANDARD.encode(signing_key.sign(&bytes).to_bytes())
        );
        (proof, resolver_for(&issuer, &signing_key))
    }

    fn resign(proof: &mut AuthorityProof) -> TestIssuerResolver {
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let bytes = crate::daemon::ability::canonical_json_bytes(&proof.canonical_material());
        proof.signature = format!(
            "ed25519:{}",
            BASE64_STANDARD.encode(signing_key.sign(&bytes).to_bytes())
        );
        resolver_for(&proof.issuer_ura, &signing_key)
    }

    fn resolver_for(issuer: &str, signing_key: &SigningKey) -> TestIssuerResolver {
        let mut keys = BTreeMap::new();
        keys.insert(issuer.to_string(), signing_key.verifying_key());
        TestIssuerResolver {
            keys,
            authorized: true,
            active: true,
        }
    }

    fn context() -> AuthorityProofVerificationContext<'static> {
        AuthorityProofVerificationContext {
            owner_user_id: "alice",
            principal_kind: PrincipalKind::Token,
            principal_id: "token-principal",
            token_id: Some("token-1"),
            callee_ura: "easynet:///r/test/device/dev",
            subject_ura: "easynet:///r/test/session/s1",
            ability_ura: "terminal.attach",
            action: AccessAction::Stream,
            nonce: None,
            canonical_hash: Some("sha256:abc"),
            audience_ura: "easynet:///r/test/device/dev",
            session_id: Some("s1"),
            session_owner_user_id: Some("alice"),
            now: DateTime::parse_from_rfc3339("2026-07-09T00:01:00Z")
                .unwrap()
                .with_timezone(&Utc),
        }
    }

    #[test]
    fn verifier_accepts_matching_signed_proof() {
        let (proof, resolver) = signed_proof();
        AuthorityProofVerifier::verify(Some(&proof), &context(), &resolver)
            .expect("proof should verify");
    }

    #[test]
    fn verifier_rejects_audience_mismatch_distinctly() {
        let (proof, resolver) = signed_proof();
        let mut context = context();
        context.audience_ura = "easynet:///r/test/device/other";
        let err = AuthorityProofVerifier::verify(Some(&proof), &context, &resolver)
            .expect_err("audience mismatch");
        assert_eq!(
            err,
            AuthorityProofDenyReason::AuthorityProofAudienceMismatch
        );
    }

    #[test]
    fn verifier_rejects_principal_kind_mismatch() {
        let (proof, resolver) = signed_proof();
        let mut context = context();
        context.principal_kind = PrincipalKind::User;
        let err = AuthorityProofVerifier::verify(Some(&proof), &context, &resolver)
            .expect_err("principal kind mismatch");
        assert_eq!(err, AuthorityProofDenyReason::AuthorityProofMismatch);
    }

    #[test]
    fn verifier_rejects_session_proof_without_followup_set() {
        let (mut proof, _resolver) = signed_proof();
        proof.allowed_followup_abilities.clear();
        let resolver = resign(&mut proof);
        let err = AuthorityProofVerifier::verify(Some(&proof), &context(), &resolver)
            .expect_err("session proof must bind follow-up ability set");
        assert_eq!(err, AuthorityProofDenyReason::AuthorityProofMismatch);
    }

    #[test]
    fn verifier_rejects_session_proof_for_disallowed_followup() {
        let (mut proof, _resolver) = signed_proof();
        proof.allowed_followup_abilities = vec!["terminal.read".to_string()];
        let resolver = resign(&mut proof);
        let err = AuthorityProofVerifier::verify(Some(&proof), &context(), &resolver)
            .expect_err("session proof must admit current follow-up ability");
        assert_eq!(err, AuthorityProofDenyReason::AuthorityProofMismatch);
    }
}
