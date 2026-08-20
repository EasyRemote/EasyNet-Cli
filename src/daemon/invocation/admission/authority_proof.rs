// EasyNet CLI — RFC-014 AuthorityProof model
// ===========================================

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use chrono::{DateTime, Utc};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::decision::{AccessAction, PrincipalKind, TokenClass};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityProof {
    pub proof_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_request_id: Option<String>,
    /// Runtime field is a canonical User URA.
    /// `owner_user_id` is for RFC-014 durable/wire compatibility only; do not reinterpret it as a bare account id or Agent identity.
    #[serde(rename = "owner_user_id")]
    pub owner_user_ura: String,
    pub principal_kind: PrincipalKind,
    pub principal_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_class: Option<TokenClass>,
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
    /// Runtime field is a canonical User URA for session-owner policy comparison.
    /// `session_owner_user_id` is a compatibility wire name and must not be used as a runtime scalar helper name.
    #[serde(
        rename = "session_owner_user_id",
        skip_serializing_if = "Option::is_none"
    )]
    pub session_owner_user_ura: Option<String>,
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
    pub(crate) fn validate_identity_contract(&self) -> Result<(), &'static str> {
        validate_user_principal_ura(&self.owner_user_ura, "owner_user_id")?;
        if self.principal_kind == PrincipalKind::User {
            validate_user_principal_ura(&self.principal_id, "principal_id")?;
        }
        if let Some(session_owner_user_ura) = self.session_owner_user_ura.as_deref() {
            validate_user_principal_ura(session_owner_user_ura, "session_owner_user_id")?;
        }
        for (field, value) in [
            ("callee_ura", self.callee_ura.as_str()),
            ("subject_ura", self.subject_ura.as_str()),
            ("issuer_ura", self.issuer_ura.as_str()),
            ("audience_ura", self.audience_ura.as_str()),
        ] {
            let identity =
                crate::core::identity::RuntimeIdentityUra::parse(value).map_err(|_| field)?;
            if identity.as_str() != value {
                return Err(field);
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn canonical_material(&self) -> Value {
        let abilities = normalized_followup_abilities(&self.allowed_followup_abilities);
        let mut value = json!({
            "profile": "easynet-authority-proof-v0",
            "proof_id": self.proof_id,
            "owner_user_id": self.owner_user_ura,
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
        if let Some(token_class) = self.token_class {
            value["token_class"] = json!(token_class);
        }
        insert_optional(&mut value, "nonce", self.nonce.as_deref());
        insert_optional(&mut value, "canonical_hash", self.canonical_hash.as_deref());
        insert_optional(&mut value, "session_id", self.session_id.as_deref());
        insert_optional(
            &mut value,
            "session_owner_user_id",
            self.session_owner_user_ura.as_deref(),
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

    #[must_use]
    pub(crate) fn matches_route_binding(&self, binding: &AuthorityProofRouteBinding<'_>) -> bool {
        self.callee_ura == binding.callee_ura
            && self.subject_ura == binding.subject_ura
            && self.ability_ura == binding.ability_ura
            && self.audience_ura == binding.audience_ura
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityProofVerificationContext<'a> {
    pub owner_user_ura: &'a str,
    pub principal_kind: PrincipalKind,
    pub principal_id: &'a str,
    pub token_id: Option<&'a str>,
    pub token_class: Option<TokenClass>,
    pub callee_ura: &'a str,
    pub subject_ura: &'a str,
    pub ability_ura: &'a str,
    pub action: AccessAction,
    pub nonce: Option<&'a str>,
    pub canonical_hash: Option<&'a str>,
    pub audience_ura: &'a str,
    pub session_id: Option<&'a str>,
    pub session_owner_user_ura: Option<&'a str>,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AuthorityProofRouteBinding<'a> {
    pub callee_ura: &'a str,
    pub subject_ura: &'a str,
    pub ability_ura: &'a str,
    pub audience_ura: &'a str,
}

pub trait AuthorityProofIssuerResolver {
    fn verifying_key_for_issuer(&self, issuer_ura: &str) -> Option<VerifyingKey>;
    fn issuer_authorized_for_owner_ura(&self, issuer_ura: &str, owner_user_ura: &str) -> bool;
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

impl AuthorityProofDenyReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AuthorityProofMissing => "AUTHORITY_PROOF_MISSING",
            Self::AuthorityProofExpired => "AUTHORITY_PROOF_EXPIRED",
            Self::AuthorityProofSignatureInvalid => "AUTHORITY_PROOF_SIGNATURE_INVALID",
            Self::AuthorityProofIssuerDenied => "AUTHORITY_PROOF_ISSUER_DENIED",
            Self::AuthorityProofAudienceMismatch => "AUTHORITY_PROOF_AUDIENCE_MISMATCH",
            Self::AuthorityProofMismatch => "AUTHORITY_PROOF_MISMATCH",
            Self::AuthorityProofRevoked => "AUTHORITY_PROOF_REVOKED",
        }
    }
}

pub struct AuthorityProofVerifier;

impl AuthorityProofVerifier {
    pub fn verify(
        proof: Option<&AuthorityProof>,
        context: &AuthorityProofVerificationContext<'_>,
        resolver: &dyn AuthorityProofIssuerResolver,
    ) -> Result<(), AuthorityProofDenyReason> {
        let proof = proof.ok_or(AuthorityProofDenyReason::AuthorityProofMissing)?;
        proof
            .validate_identity_contract()
            .map_err(|_| AuthorityProofDenyReason::AuthorityProofMismatch)?;
        validate_verification_context_identity(context)?;
        verify_not_expired(proof, context.now)?;
        verify_invocation_binding(proof, context)?;
        verify_signature(proof, resolver)?;
        if !resolver.issuer_authorized_for_owner_ura(&proof.issuer_ura, &proof.owner_user_ura) {
            return Err(AuthorityProofDenyReason::AuthorityProofIssuerDenied);
        }
        if !resolver.referenced_authority_active(proof) {
            return Err(AuthorityProofDenyReason::AuthorityProofRevoked);
        }
        Ok(())
    }
}

fn validate_verification_context_identity(
    context: &AuthorityProofVerificationContext<'_>,
) -> Result<(), AuthorityProofDenyReason> {
    validate_user_principal_ura(context.owner_user_ura, "owner_user_ura")
        .map_err(|_| AuthorityProofDenyReason::AuthorityProofMismatch)?;
    if context.principal_kind == PrincipalKind::User {
        validate_user_principal_ura(context.principal_id, "principal_id")
            .map_err(|_| AuthorityProofDenyReason::AuthorityProofMismatch)?;
    }
    if let Some(session_owner_user_ura) = context.session_owner_user_ura {
        validate_user_principal_ura(session_owner_user_ura, "session_owner_user_id")
            .map_err(|_| AuthorityProofDenyReason::AuthorityProofMismatch)?;
    }
    for value in [
        context.callee_ura,
        context.subject_ura,
        context.audience_ura,
    ] {
        let identity = crate::core::identity::RuntimeIdentityUra::parse(value)
            .map_err(|_| AuthorityProofDenyReason::AuthorityProofMismatch)?;
        if identity.as_str() != value {
            return Err(AuthorityProofDenyReason::AuthorityProofMismatch);
        }
    }
    Ok(())
}

fn validate_nonzero_user_id(value: &str, field: &'static str) -> Result<(), &'static str> {
    if value.trim().is_empty() || crate::core::identity::is_all_zero_principal_id(value) {
        return Err(field);
    }
    Ok(())
}

fn validate_user_principal_ura(value: &str, field: &'static str) -> Result<(), &'static str> {
    validate_nonzero_user_id(value, field)?;
    let parsed = crate::core::ura::parse_ura(value).map_err(|_| field)?;
    if parsed.kind != crate::core::ura::URAKind::User || parsed.user_id().is_none() {
        return Err(field);
    }
    Ok(())
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
    let route_binding = AuthorityProofRouteBinding {
        callee_ura: context.callee_ura,
        subject_ura: context.subject_ura,
        ability_ura: context.ability_ura,
        audience_ura: context.audience_ura,
    };
    if proof.audience_ura != context.audience_ura {
        return Err(AuthorityProofDenyReason::AuthorityProofAudienceMismatch);
    }
    if proof.owner_user_ura != context.owner_user_ura
        || proof.principal_kind != context.principal_kind
        || proof.principal_id != context.principal_id
        || proof.token_id.as_deref() != context.token_id
        || proof.token_class != context.token_class
        || !proof.matches_route_binding(&route_binding)
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
    verify_session_binding_facts(proof, context)?;
    if proof.session_id.is_some() {
        let allowed = normalized_followup_abilities(&proof.allowed_followup_abilities);
        if allowed.is_empty() {
            return Err(AuthorityProofDenyReason::AuthorityProofMismatch);
        }
        if !allowed.iter().any(|ability| ability == context.ability_ura) {
            return Err(AuthorityProofDenyReason::AuthorityProofMismatch);
        }
    }
    if request_scoped_one_time_authority_proof(proof) && !proof_binds_invocation_identity(proof) {
        return Err(AuthorityProofDenyReason::AuthorityProofMismatch);
    }
    Ok(())
}

fn verify_session_binding_facts(
    proof: &AuthorityProof,
    context: &AuthorityProofVerificationContext<'_>,
) -> Result<(), AuthorityProofDenyReason> {
    if proof.session_id.is_some() {
        let proof_owner = proof
            .session_owner_user_ura
            .as_deref()
            .map(str::trim)
            .filter(|owner| !owner.is_empty())
            .ok_or(AuthorityProofDenyReason::AuthorityProofMismatch)?;
        let context_owner = context
            .session_owner_user_ura
            .map(str::trim)
            .filter(|owner| !owner.is_empty())
            .ok_or(AuthorityProofDenyReason::AuthorityProofMismatch)?;
        if proof_owner != context_owner {
            return Err(AuthorityProofDenyReason::AuthorityProofMismatch);
        }
        return Ok(());
    }
    if proof.session_owner_user_ura.as_deref().is_some()
        && proof.session_owner_user_ura.as_deref() != context.session_owner_user_ura
    {
        return Err(AuthorityProofDenyReason::AuthorityProofMismatch);
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

pub(crate) fn request_scoped_one_time_authority_proof(proof: &AuthorityProof) -> bool {
    proof.permission_request_id.is_some() && proof.grant_id.is_none() && proof.session_id.is_none()
}

fn proof_binds_invocation_identity(proof: &AuthorityProof) -> bool {
    proof
        .nonce
        .as_deref()
        .map(str::trim)
        .is_some_and(|nonce| !nonce.is_empty())
        || proof
            .canonical_hash
            .as_deref()
            .map(str::trim)
            .is_some_and(|hash| !hash.is_empty())
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

    const ALICE_USER_URA: &str = "easynet:///r/test/user/alice";

    #[test]
    fn canonical_material_sorts_and_deduplicates_followup_abilities() {
        let proof = AuthorityProof {
            proof_id: "proof-1".to_string(),
            grant_id: Some("grant-1".to_string()),
            permission_request_id: None,
            owner_user_ura: ALICE_USER_URA.to_string(),
            principal_kind: PrincipalKind::Token,
            principal_id: "token-principal".to_string(),
            token_id: Some("token-1".to_string()),
            token_class: Some(TokenClass::HubLink),
            callee_ura: "easynet:///r/test/device/dev".to_string(),
            subject_ura: crate::core::ura::resource_dot_ura("test", "user.alice", "session/s1"),
            ability_ura: "terminal.attach".to_string(),
            action: AccessAction::Stream,
            nonce: None,
            canonical_hash: Some("sha256:abc".to_string()),
            session_id: Some("s1".to_string()),
            session_owner_user_ura: Some(ALICE_USER_URA.to_string()),
            allowed_followup_abilities: vec![
                "terminal.read".to_string(),
                "terminal.attach".to_string(),
                "terminal.read".to_string(),
            ],
            session_expires_at: Some("2026-07-09T01:00:00Z".to_string()),
            issued_at: "2026-07-09T00:00:00Z".to_string(),
            expires_at: "2026-07-09T00:05:00Z".to_string(),
            issuer_ura: ALICE_USER_URA.to_string(),
            audience_ura: "easynet:///r/test/device/dev".to_string(),
            signature: "sig".to_string(),
        };
        assert_eq!(
            proof.canonical_material()["allowed_followup_abilities"],
            json!(["terminal.attach", "terminal.read"])
        );
        assert_eq!(proof.canonical_material()["token_class"], json!("hub_link"));
    }

    #[test]
    fn authority_proof_deserialization_rejects_unknown_fields() {
        let raw = json!({
            "proof_id": "proof-1",
            "grant_id": "grant-1",
            "owner_user_id": ALICE_USER_URA,
            "principal_kind": "token",
            "principal_id": "token-principal",
            "token_id": "token-1",
            "token_class": "hub_link",
            "callee_ura": "easynet:///r/test/agent/device.dev.terminal",
            "subject_ura": "easynet:///r/test/resource/user.alice/session/s1",
            "ability_ura": "terminal.attach",
            "action": "stream",
            "canonical_hash": "sha256:abc",
            "session_id": "s1",
            "session_owner_user_id": ALICE_USER_URA,
            "allowed_followup_abilities": ["terminal.attach"],
            "session_expires_at": "2026-07-09T01:00:00Z",
            "issued_at": "2026-07-09T00:00:00Z",
            "expires_at": "2026-07-09T00:05:00Z",
            "issuer_ura": "easynet:///r/test/user/alice",
            "audience_ura": "easynet:///r/test/device/dev",
            "signature": "ed25519:signature",
            "legacy_scope": "compat-carrier"
        });

        let error = serde_json::from_value::<AuthorityProof>(raw)
            .expect_err("authority proof must reject unknown fields");
        assert!(
            error.to_string().contains("unknown field `legacy_scope`"),
            "error should name the noncanonical proof field: {error}"
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

        fn issuer_authorized_for_owner_ura(
            &self,
            _issuer_ura: &str,
            _owner_user_ura: &str,
        ) -> bool {
            self.authorized
        }

        fn referenced_authority_active(&self, _proof: &AuthorityProof) -> bool {
            self.active
        }
    }

    fn signed_proof() -> (AuthorityProof, TestIssuerResolver) {
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let issuer = ALICE_USER_URA.to_string();
        let mut proof = AuthorityProof {
            proof_id: "proof-1".to_string(),
            grant_id: Some("grant-1".to_string()),
            permission_request_id: None,
            owner_user_ura: ALICE_USER_URA.to_string(),
            principal_kind: PrincipalKind::Token,
            principal_id: "token-principal".to_string(),
            token_id: Some("token-1".to_string()),
            token_class: Some(TokenClass::HubLink),
            callee_ura: "easynet:///r/test/device/dev".to_string(),
            subject_ura: crate::core::ura::resource_dot_ura("test", "user.alice", "session/s1"),
            ability_ura: "terminal.attach".to_string(),
            action: AccessAction::Stream,
            nonce: None,
            canonical_hash: Some("sha256:abc".to_string()),
            session_id: Some("s1".to_string()),
            session_owner_user_ura: Some(ALICE_USER_URA.to_string()),
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
            owner_user_ura: ALICE_USER_URA,
            principal_kind: PrincipalKind::Token,
            principal_id: "token-principal",
            token_id: Some("token-1"),
            token_class: Some(TokenClass::HubLink),
            callee_ura: "easynet:///r/test/device/dev",
            subject_ura: "easynet:///r/test/resource/user.alice/session/s1",
            ability_ura: "terminal.attach",
            action: AccessAction::Stream,
            nonce: None,
            canonical_hash: Some("sha256:abc"),
            audience_ura: "easynet:///r/test/device/dev",
            session_id: Some("s1"),
            session_owner_user_ura: Some(ALICE_USER_URA),
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
    fn verifier_rejects_bare_context_owner_user_id() {
        let (proof, resolver) = signed_proof();
        let mut context = context();
        context.owner_user_ura = "alice";

        let err = AuthorityProofVerifier::verify(Some(&proof), &context, &resolver)
            .expect_err("runtime authority context owner must be a User URA");

        assert_eq!(err, AuthorityProofDenyReason::AuthorityProofMismatch);
    }

    #[test]
    fn verifier_rejects_bare_context_session_owner_user_id() {
        let (proof, resolver) = signed_proof();
        let mut context = context();
        context.session_owner_user_ura = Some("alice");

        let err = AuthorityProofVerifier::verify(Some(&proof), &context, &resolver)
            .expect_err("runtime authority context session owner must be a User URA");

        assert_eq!(err, AuthorityProofDenyReason::AuthorityProofMismatch);
    }

    #[test]
    fn verifier_rejects_token_class_mismatch() {
        let (proof, resolver) = signed_proof();
        let mut context = context();
        context.token_class = Some(TokenClass::BrowserSession);
        let err = AuthorityProofVerifier::verify(Some(&proof), &context, &resolver)
            .expect_err("token class mismatch");
        assert_eq!(err, AuthorityProofDenyReason::AuthorityProofMismatch);
    }

    #[test]
    fn verifier_rejects_all_zero_owner_and_session_identity_facts() {
        let (mut proof, _resolver) = signed_proof();
        proof.owner_user_ura = crate::core::identity::ALL_ZERO_PRINCIPAL_ID.to_string();
        let resolver = resign(&mut proof);
        assert_eq!(
            AuthorityProofVerifier::verify(Some(&proof), &context(), &resolver),
            Err(AuthorityProofDenyReason::AuthorityProofMismatch)
        );

        let (mut proof, _resolver) = signed_proof();
        proof.session_owner_user_ura =
            Some(crate::core::identity::ALL_ZERO_PRINCIPAL_ID.to_string());
        let resolver = resign(&mut proof);
        assert_eq!(
            AuthorityProofVerifier::verify(Some(&proof), &context(), &resolver),
            Err(AuthorityProofDenyReason::AuthorityProofMismatch)
        );
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
    fn verifier_rejects_session_proof_without_session_owner_fact() {
        let (mut proof, _resolver) = signed_proof();
        proof.session_owner_user_ura = None;
        let resolver = resign(&mut proof);
        let err = AuthorityProofVerifier::verify(Some(&proof), &context(), &resolver)
            .expect_err("session proof must bind session owner");
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

    #[test]
    fn verifier_rejects_unbound_request_scoped_one_time_proof() {
        let (mut proof, _resolver) = signed_proof();
        proof.grant_id = None;
        proof.permission_request_id = Some("req-1".to_string());
        proof.nonce = None;
        proof.canonical_hash = None;
        proof.session_id = None;
        proof.session_owner_user_ura = None;
        proof.allowed_followup_abilities.clear();
        proof.session_expires_at = None;
        let resolver = resign(&mut proof);

        let err = AuthorityProofVerifier::verify(Some(&proof), &context(), &resolver)
            .expect_err("request-scoped one-time proof must bind invocation identity");
        assert_eq!(err, AuthorityProofDenyReason::AuthorityProofMismatch);
    }
}
