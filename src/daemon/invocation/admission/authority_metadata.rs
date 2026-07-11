// EasyNet CLI — invocation authority metadata core
// =================================================
//
// File: src/daemon/invocation/admission/authority_metadata.rs
// Description: Canonical delegated/session authority payload validation and
//              post-admission runtime projection.
//
// Protocol Responsibility
// -----------------------
// Own the EasyNet-Cli side of daemon admission authority metadata. Signature
// and trust-anchor verification remain in admission; runtime handlers consume
// only authority payloads that have passed that gate.
//
// Implementation Approach
// -----------------------
// Authority payloads are typed Rust domain objects. Canonical bytes are derived
// through the daemon's canonical JSON helper, matching admission verification.

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::daemon::ability::canonical_json_bytes;

pub(crate) const DELEGATION_METADATA_KEY: &str = "x-easynet-delegation";
pub(crate) const SESSION_AUTHORITY_METADATA_KEY: &str = "x-easynet-session-authority";
pub(crate) const REASON_AUTHORITY_FORMAT_INVALID: &str = "AUTHORITY_FORMAT_INVALID";
pub(crate) const REASON_AUTHORITY_EXPIRED: &str = "AUTHORITY_EXPIRED";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuthorityMetadataError {
    reason: &'static str,
    message: String,
}

impl AuthorityMetadataError {
    fn new(reason: &'static str, message: impl Into<String>) -> Self {
        Self {
            reason,
            message: message.into(),
        }
    }

    pub(crate) fn reason(&self) -> &'static str {
        self.reason
    }

    pub(crate) fn status_message(&self) -> String {
        format!("{}: {}", self.reason, self.message)
    }
}

impl std::fmt::Display for AuthorityMetadataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.reason, self.message)
    }
}

impl std::error::Error for AuthorityMetadataError {}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct DelegationPayload {
    pub(crate) issuer_ura: String,
    pub(crate) subject_ura: String,
    pub(crate) caller_ura: String,
    pub(crate) audience: String,
    pub(crate) scopes: Vec<String>,
    pub(crate) issued_at_ms: i64,
    pub(crate) expires_at_ms: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct SessionAuthorityPayload {
    pub(crate) issuer_ura: String,
    pub(crate) session_id: String,
    pub(crate) session_owner_user_id: String,
    pub(crate) creator_principal_id: String,
    pub(crate) callee_ura: String,
    pub(crate) subject_ura: String,
    pub(crate) audience: String,
    pub(crate) scopes: Vec<String>,
    pub(crate) allowed_actions: Vec<String>,
    pub(crate) allowed_followup_abilities: Vec<String>,
    pub(crate) issued_at_ms: i64,
    pub(crate) expires_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct SignedSessionAuthorityWire {
    payload: SessionAuthorityPayload,
    signature: String,
}

/// Project an authority that has already passed the transport admission gate
/// into the generic runtime context. This function deliberately does not
/// verify cryptography: the caller is the post-admission LocalRuntime
/// adapter, and admission remains the single signature/trust authority.
pub(crate) fn project_admitted_session_authority(
    metadata: &HashMap<String, String>,
) -> Result<Option<SessionAuthorityPayload>, AuthorityMetadataError> {
    let Some(raw) = metadata
        .get(SESSION_AUTHORITY_METADATA_KEY)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let wire_bytes = BASE64_STANDARD.decode(raw).map_err(|err| {
        AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            format!("admitted session authority base64 decode failed: {err}"),
        )
    })?;
    let wire: SignedSessionAuthorityWire = serde_json::from_slice(&wire_bytes).map_err(|err| {
        AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            format!("admitted session authority JSON parse failed: {err}"),
        )
    })?;
    if wire.signature.trim().is_empty() {
        return Err(AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            "admitted session authority signature is empty",
        ));
    }
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    validate_session_authority_payload_shape(&wire.payload, Some(now_ms))?;
    Ok(Some(wire.payload))
}

pub(crate) fn canonical_authority_payload_bytes<T: Serialize>(
    payload: &T,
) -> Result<Vec<u8>, AuthorityMetadataError> {
    let value = serde_json::to_value(payload).map_err(|err| {
        AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            format!("authority payload canonical value marshal failed: {err}"),
        )
    })?;
    Ok(canonical_json_bytes(&value))
}

pub(crate) fn validate_delegation_payload_shape(
    payload: &DelegationPayload,
    now_ms: Option<i64>,
) -> Result<(), AuthorityMetadataError> {
    if payload.issuer_ura.trim().is_empty()
        || payload.subject_ura.trim().is_empty()
        || payload.caller_ura.trim().is_empty()
        || payload.audience.trim().is_empty()
        || payload.scopes.is_empty()
        || payload.scopes.iter().any(|scope| scope.trim().is_empty())
    {
        return Err(AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            "authority payload must carry issuer, subject, caller, audience, and at least one non-empty scope",
        ));
    }
    validate_expiry(
        "authority",
        payload.issued_at_ms,
        payload.expires_at_ms,
        now_ms,
    )
}

pub(crate) fn validate_session_authority_payload_shape(
    payload: &SessionAuthorityPayload,
    now_ms: Option<i64>,
) -> Result<(), AuthorityMetadataError> {
    if payload.issuer_ura.trim().is_empty()
        || payload.session_id.trim().is_empty()
        || payload.session_owner_user_id.trim().is_empty()
        || payload.creator_principal_id.trim().is_empty()
        || payload.callee_ura.trim().is_empty()
        || payload.subject_ura.trim().is_empty()
        || payload.audience.trim().is_empty()
        || payload.scopes.is_empty()
        || payload.scopes.iter().any(|scope| scope.trim().is_empty())
        || payload.allowed_actions.is_empty()
        || payload
            .allowed_actions
            .iter()
            .any(|action| action.trim().is_empty())
        || payload.allowed_followup_abilities.is_empty()
        || payload
            .allowed_followup_abilities
            .iter()
            .any(|ability| ability.trim().is_empty())
    {
        return Err(AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            "session authority must carry issuer, session id, owner, creator principal, callee, subject, audience, scopes, allowed actions, and follow-up abilities",
        ));
    }
    let subject_kind = authority_subject_kind(&payload.subject_ura);
    if !matches!(
        subject_kind,
        AuthoritySubjectKind::User | AuthoritySubjectKind::Session
    ) {
        return Err(AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            format!(
                "session authority subject_ura `{}` must be a canonical user or session subject",
                payload.subject_ura
            ),
        ));
    }
    if subject_kind == AuthoritySubjectKind::User {
        let parsed = crate::core::ura::parse_ura(payload.subject_ura.trim()).map_err(|err| {
            AuthorityMetadataError::new(
                REASON_AUTHORITY_FORMAT_INVALID,
                format!("session authority subject_ura parse failed: {err}"),
            )
        })?;
        if parsed.user_id() != Some(payload.session_owner_user_id.as_str()) {
            return Err(AuthorityMetadataError::new(
                REASON_AUTHORITY_FORMAT_INVALID,
                format!(
                    "session authority user subject must match session_owner_user_id `{}`",
                    payload.session_owner_user_id
                ),
            ));
        }
    } else if subject_kind == AuthoritySubjectKind::Session {
        let parsed = crate::core::ura::parse_ura(payload.subject_ura.trim()).map_err(|err| {
            AuthorityMetadataError::new(
                REASON_AUTHORITY_FORMAT_INVALID,
                format!("session authority subject_ura parse failed: {err}"),
            )
        })?;
        let (owner_user_id, session_id) =
            canonical_session_resource_parts(&parsed).ok_or_else(|| {
                AuthorityMetadataError::new(
                REASON_AUTHORITY_FORMAT_INVALID,
                "session authority subject_ura must name one canonical user-owned session resource",
            )
            })?;
        if owner_user_id != payload.session_owner_user_id || session_id != payload.session_id {
            return Err(AuthorityMetadataError::new(
                REASON_AUTHORITY_FORMAT_INVALID,
                format!(
                    "session authority subject_ura owner/session must match session_owner_user_id `{}` and session_id `{}`",
                    payload.session_owner_user_id, payload.session_id
                ),
            ));
        }
    }
    validate_expiry(
        "session authority",
        payload.issued_at_ms,
        payload.expires_at_ms,
        now_ms,
    )
}

pub(crate) fn authority_subject_kind(subject_ura: &str) -> AuthoritySubjectKind {
    let Ok(parsed) = crate::core::ura::parse_ura(subject_ura.trim()) else {
        return AuthoritySubjectKind::Other;
    };
    match parsed.kind {
        crate::core::ura::URAKind::User => AuthoritySubjectKind::User,
        crate::core::ura::URAKind::Resource
            if canonical_session_resource_parts(&parsed).is_some() =>
        {
            AuthoritySubjectKind::Session
        }
        _ => AuthoritySubjectKind::Other,
    }
}

fn canonical_session_resource_parts(parsed: &crate::core::ura::ParsedURA) -> Option<(&str, &str)> {
    let Some(owner_user_id) = parsed
        .resource_owner_id()
        .and_then(|owner| owner.strip_prefix("user."))
    else {
        return None;
    };
    if owner_user_id.is_empty() || owner_user_id.contains('.') {
        return None;
    }

    let session_id = parsed
        .resource_path()
        .and_then(|path| path.strip_prefix("session/"))
        .filter(|session_id| !session_id.is_empty() && !session_id.contains('/'))?;
    Some((owner_user_id, session_id))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthoritySubjectKind {
    User,
    Session,
    Other,
}

fn validate_expiry(
    label: &str,
    issued_at_ms: i64,
    expires_at_ms: i64,
    now_ms: Option<i64>,
) -> Result<(), AuthorityMetadataError> {
    if expires_at_ms <= issued_at_ms {
        return Err(AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            format!("{label} expires_at_ms must be greater than issued_at_ms"),
        ));
    }
    if let Some(now_ms) = now_ms {
        if now_ms >= expires_at_ms {
            return Err(AuthorityMetadataError::new(
                REASON_AUTHORITY_EXPIRED,
                format!("{label} expired at {expires_at_ms}ms (now {now_ms}ms)"),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_payload() -> SessionAuthorityPayload {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        SessionAuthorityPayload {
            issuer_ura: "easynet:///r/example/agent/backend".into(),
            session_id: "session-1".into(),
            session_owner_user_id: "alice".into(),
            creator_principal_id: "easynet:///r/example/agent/backend".into(),
            callee_ura: "easynet:///r/example/device/dev-a".into(),
            subject_ura: "easynet:///r/example/resource/user.alice/session/session-1".into(),
            audience: "easynet:///r/example/device/dev-a".into(),
            scopes: vec!["device.observe.*".into()],
            allowed_actions: vec!["read".into()],
            allowed_followup_abilities: vec!["device.observe.health".into()],
            issued_at_ms: now_ms,
            expires_at_ms: now_ms + 60_000,
        }
    }

    #[test]
    fn delegation_payload_has_one_canonical_signing_representation() {
        let payload = DelegationPayload {
            issuer_ura: "easynet:///r/example/user/alice".into(),
            subject_ura: "easynet:///r/example/user/alice".into(),
            caller_ura: "easynet:///r/example/agent/backend".into(),
            audience: "easynet:///r/example/device/dev-a".into(),
            scopes: vec!["device.observe.*".into()],
            issued_at_ms: 1000,
            expires_at_ms: 2000,
        };
        validate_delegation_payload_shape(&payload, None).unwrap();
        let canonical = canonical_authority_payload_bytes(&payload).unwrap();
        assert_eq!(
            String::from_utf8(canonical).unwrap(),
            r#"{"audience":"easynet:///r/example/device/dev-a","caller_ura":"easynet:///r/example/agent/backend","expires_at_ms":2000,"issued_at_ms":1000,"issuer_ura":"easynet:///r/example/user/alice","scopes":["device.observe.*"],"subject_ura":"easynet:///r/example/user/alice"}"#
        );
    }

    #[test]
    fn post_admission_projection_preserves_the_verified_payload() {
        let payload = session_payload();
        let expected = payload.clone();
        let wire = serde_json::json!({
            "payload": payload,
            "signature": "c2Vzc2lvbi1zaWduYXR1cmU="
        });
        let metadata = HashMap::from([(
            SESSION_AUTHORITY_METADATA_KEY.to_string(),
            BASE64_STANDARD.encode(serde_json::to_vec(&wire).unwrap()),
        )]);

        let projected = project_admitted_session_authority(&metadata)
            .unwrap()
            .expect("session authority must be projected");
        assert_eq!(projected, expected);
    }

    #[test]
    fn session_authority_binds_subject_resource_to_declared_owner_and_session() {
        for subject_ura in [
            "easynet:///r/example/resource/user.bob/session/session-1",
            "easynet:///r/example/resource/user.alice/session/session-2",
            "easynet:///r/example/session/session-1",
            "easynet:///r/example/user/bob",
        ] {
            let mut payload = session_payload();
            payload.subject_ura = subject_ura.into();
            let err = validate_session_authority_payload_shape(&payload, None).unwrap_err();
            assert_eq!(err.reason(), REASON_AUTHORITY_FORMAT_INVALID);
        }

        let mut matching_user_subject = session_payload();
        matching_user_subject.subject_ura = "easynet:///r/example/user/alice".into();
        validate_session_authority_payload_shape(&matching_user_subject, None)
            .expect("the declared session owner remains a canonical user subject");
    }

    #[test]
    fn session_authority_requires_followup_binding_fields() {
        let mut payload = session_payload();
        payload.allowed_followup_abilities.clear();
        let err = validate_session_authority_payload_shape(&payload, None).unwrap_err();
        assert_eq!(err.reason(), REASON_AUTHORITY_FORMAT_INVALID);
    }

    #[test]
    fn authority_subject_kind_accepts_only_canonical_user_or_session_resources() {
        assert_eq!(
            authority_subject_kind("easynet:///r/example/user/alice"),
            AuthoritySubjectKind::User
        );
        assert_eq!(
            authority_subject_kind("easynet:///r/example/resource/user.alice/session/session-1"),
            AuthoritySubjectKind::Session
        );
        assert_eq!(
            authority_subject_kind("easynet:///r/example/session/session-1"),
            AuthoritySubjectKind::Other,
            "the Axon URA grammar has no top-level session role"
        );
        assert_eq!(
            authority_subject_kind("easynet:///r/example/resource/device.dev-a/session/session-1"),
            AuthoritySubjectKind::Other,
            "session resources are owned by the session user"
        );
        assert_eq!(
            authority_subject_kind(
                "easynet:///r/example/resource/user.alice/session/session-1/child"
            ),
            AuthoritySubjectKind::Other,
            "a session subject names one session resource, not a descendant path"
        );
    }
}
