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

pub(crate) const DELEGATION_METADATA_KEY: &str = "x-runtime-delegation";
pub(crate) const SESSION_AUTHORITY_METADATA_KEY: &str = "x-runtime-session-authority";
pub(crate) const REASON_AUTHORITY_FORMAT_INVALID: &str = "AUTHORITY_FORMAT_INVALID";
pub(crate) const REASON_AUTHORITY_EXPIRED: &str = "AUTHORITY_EXPIRED";
pub(crate) const REASON_AUTHORITY_CLOCK_UNAVAILABLE: &str = "AUTHORITY_CLOCK_UNAVAILABLE";

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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
struct SignedSessionAuthorityWire {
    payload: SessionAuthorityPayload,
    signature: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedDelegationAuthorityWire {
    payload: DelegationPayload,
    signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InvocationAuthorityMetadata {
    Delegation(DelegationPayload),
    Session(SessionAuthorityPayload),
}

/// Project invocation authority metadata for pre-transport shape validation.
///
/// This deliberately validates only canonical payload shape, expiry, and
/// signature presence. Cryptographic verification remains owned by admission;
/// consumers use this projection only to reject contradictory public tuple
/// facts before daemon I/O.
pub(crate) fn project_invocation_authority_metadata_shape(
    metadata: &HashMap<String, String>,
) -> Result<Option<InvocationAuthorityMetadata>, AuthorityMetadataError> {
    let delegation_raw = metadata
        .get(DELEGATION_METADATA_KEY)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let session_raw = metadata
        .get(SESSION_AUTHORITY_METADATA_KEY)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match (delegation_raw, session_raw) {
        (Some(_), Some(_)) => Err(AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            "invocation authority metadata is ambiguous",
        )),
        (Some(raw), None) => project_delegation_authority_shape(raw)
            .map(|payload| Some(InvocationAuthorityMetadata::Delegation(payload))),
        (None, Some(raw)) => project_session_authority_shape(raw)
            .map(|payload| Some(InvocationAuthorityMetadata::Session(payload))),
        (None, None) => Ok(None),
    }
}

fn project_delegation_authority_shape(
    raw: &str,
) -> Result<DelegationPayload, AuthorityMetadataError> {
    let wire_bytes = BASE64_STANDARD.decode(raw).map_err(|err| {
        AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            format!("delegation authority base64 decode failed: {err}"),
        )
    })?;
    let wire: SignedDelegationAuthorityWire =
        serde_json::from_slice(&wire_bytes).map_err(|err| {
            AuthorityMetadataError::new(
                REASON_AUTHORITY_FORMAT_INVALID,
                format!("delegation authority JSON parse failed: {err}"),
            )
        })?;
    if wire.signature.trim().is_empty() {
        return Err(AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            "delegation authority signature is empty",
        ));
    }
    let now_ms = current_unix_epoch_millis()?;
    validate_delegation_payload_shape(&wire.payload, Some(now_ms))?;
    Ok(wire.payload)
}

fn project_session_authority_shape(
    raw: &str,
) -> Result<SessionAuthorityPayload, AuthorityMetadataError> {
    let wire_bytes = BASE64_STANDARD.decode(raw).map_err(|err| {
        AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            format!("session authority base64 decode failed: {err}"),
        )
    })?;
    let wire: SignedSessionAuthorityWire = serde_json::from_slice(&wire_bytes).map_err(|err| {
        AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            format!("session authority JSON parse failed: {err}"),
        )
    })?;
    if wire.signature.trim().is_empty() {
        return Err(AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            "session authority signature is empty",
        ));
    }
    let now_ms = current_unix_epoch_millis()?;
    validate_session_authority_payload_shape(&wire.payload, Some(now_ms))?;
    Ok(wire.payload)
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
    project_session_authority_shape(raw).map(Some)
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
    reject_all_zero_authority_fields(
        "authority",
        &[
            ("issuer_ura", &payload.issuer_ura),
            ("subject_ura", &payload.subject_ura),
            ("caller_ura", &payload.caller_ura),
            ("audience", &payload.audience),
        ],
    )?;
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
    reject_all_zero_authority_fields(
        "session authority",
        &[
            ("issuer_ura", &payload.issuer_ura),
            ("session_owner_user_id", &payload.session_owner_user_id),
            ("creator_principal_id", &payload.creator_principal_id),
            ("callee_ura", &payload.callee_ura),
            ("subject_ura", &payload.subject_ura),
            ("audience", &payload.audience),
        ],
    )?;
    if crate::core::identity::is_retired_invocation_history_subject(&payload.subject_ura) {
        return Err(AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            "session authority subject_ura uses retired invocation-history subject; use runtime-state/read",
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

fn reject_all_zero_authority_fields(
    label: &str,
    fields: &[(&str, &str)],
) -> Result<(), AuthorityMetadataError> {
    for (field, value) in fields {
        if crate::core::identity::contains_all_zero_principal_placeholder(value) {
            return Err(AuthorityMetadataError::new(
                REASON_AUTHORITY_FORMAT_INVALID,
                format!("{label} {field} must not be all-zero"),
            ));
        }
    }
    Ok(())
}

pub(crate) fn authority_subject_kind(subject_ura: &str) -> AuthoritySubjectKind {
    if crate::core::identity::is_retired_invocation_history_subject(subject_ura) {
        return AuthoritySubjectKind::Other;
    }
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
    let owner_user_id = parsed
        .resource_owner_id()
        .and_then(|owner| owner.strip_prefix("user."))?;
    if owner_user_id.is_empty() || owner_user_id.contains('.') {
        return None;
    }

    let session_id = parsed
        .resource_path()
        .and_then(|path| path.strip_prefix("session/"))
        .filter(|session_id| !session_id.is_empty() && !session_id.contains('/'))?;
    Some((owner_user_id, session_id))
}

pub(crate) fn session_authority_admits_subject(
    payload: &SessionAuthorityPayload,
    subject: &str,
) -> bool {
    if crate::core::identity::is_retired_invocation_history_subject(subject) {
        return false;
    }
    if payload.subject_ura == subject {
        return true;
    }
    let Ok(parsed) = crate::core::ura::parse_ura(subject) else {
        return false;
    };
    if parsed.kind != crate::core::ura::URAKind::Resource {
        return false;
    }
    let Some(owner_id) = parsed.resource_owner_id() else {
        return false;
    };
    resource_owner_matches_session_owner(owner_id, &payload.session_owner_user_id)
}

fn resource_owner_matches_session_owner(owner_id: &str, session_owner_user_id: &str) -> bool {
    let session_owner_user_id = session_owner_user_id.trim();
    if session_owner_user_id.is_empty() {
        return false;
    }
    if let Some(user_id) = owner_id.strip_prefix("user.") {
        return user_id == session_owner_user_id;
    }
    owner_id
        .strip_prefix("agent.")
        .and_then(|rest| rest.split_once('.').map(|(user_id, _)| user_id))
        .is_some_and(|user_id| user_id == session_owner_user_id)
}

pub(crate) fn authority_audience_admits(audience: &str, callee: &str) -> bool {
    let audience = audience.trim();
    let callee = callee.trim();
    audience == "*" || audience == callee || audience.ends_with('/') && callee.starts_with(audience)
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

fn current_unix_epoch_millis() -> Result<i64, AuthorityMetadataError> {
    unix_epoch_millis(SystemTime::now())
}

fn unix_epoch_millis(now: SystemTime) -> Result<i64, AuthorityMetadataError> {
    let duration = now.duration_since(UNIX_EPOCH).map_err(|err| {
        AuthorityMetadataError::new(
            REASON_AUTHORITY_CLOCK_UNAVAILABLE,
            format!("authority clock is before the Unix epoch: {err}"),
        )
    })?;
    i64::try_from(duration.as_millis()).map_err(|err| {
        AuthorityMetadataError::new(
            REASON_AUTHORITY_CLOCK_UNAVAILABLE,
            format!("authority clock value exceeds i64 milliseconds: {err}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

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

    fn delegation_payload() -> DelegationPayload {
        DelegationPayload {
            issuer_ura: "easynet:///r/example/user/alice".into(),
            subject_ura: "easynet:///r/example/user/alice".into(),
            caller_ura: "easynet:///r/example/agent/backend".into(),
            audience: "easynet:///r/example/device/dev-a".into(),
            scopes: vec!["device.observe.*".into()],
            issued_at_ms: 1000,
            expires_at_ms: 4_102_444_800_000,
        }
    }

    fn encode_authority_wire(wire: serde_json::Value) -> String {
        BASE64_STANDARD.encode(serde_json::to_vec(&wire).expect("authority wire serializes"))
    }

    #[test]
    fn delegation_payload_has_one_canonical_signing_representation() {
        let mut payload = delegation_payload();
        payload.expires_at_ms = 2000;
        validate_delegation_payload_shape(&payload, None).unwrap();
        let canonical = canonical_authority_payload_bytes(&payload).unwrap();
        assert_eq!(
            String::from_utf8(canonical).unwrap(),
            r#"{"audience":"easynet:///r/example/device/dev-a","caller_ura":"easynet:///r/example/agent/backend","expires_at_ms":2000,"issued_at_ms":1000,"issuer_ura":"easynet:///r/example/user/alice","scopes":["device.observe.*"],"subject_ura":"easynet:///r/example/user/alice"}"#
        );
    }

    #[test]
    fn delegation_payload_rejects_all_zero_principal_placeholders() {
        let payload = DelegationPayload {
            issuer_ura: "easynet:///r/example/user/alice".into(),
            subject_ura: "easynet:///r/example/user/00000000-0000-0000-0000-000000000000".into(),
            caller_ura: "easynet:///r/example/agent/backend".into(),
            audience: "easynet:///r/example/device/dev-a".into(),
            scopes: vec!["device.observe.*".into()],
            issued_at_ms: 1000,
            expires_at_ms: 2000,
        };

        let err = validate_delegation_payload_shape(&payload, None)
            .expect_err("all-zero authority placeholders must be rejected");
        assert_eq!(err.reason(), REASON_AUTHORITY_FORMAT_INVALID);
        assert!(
            err.to_string().contains("subject_ura must not be all-zero"),
            "{err}"
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
    fn session_authority_rejects_unknown_payload_fields() {
        let mut payload =
            serde_json::to_value(session_payload()).expect("session authority payload serializes");
        payload
            .as_object_mut()
            .expect("session authority payload is object")
            .insert(
                "retired_subject_locator".to_string(),
                serde_json::json!("compat-carrier"),
            );
        let wire = serde_json::json!({
            "payload": payload,
            "signature": "c2Vzc2lvbi1zaWduYXR1cmU="
        });
        let metadata = HashMap::from([(
            SESSION_AUTHORITY_METADATA_KEY.to_string(),
            encode_authority_wire(wire),
        )]);

        let err = project_admitted_session_authority(&metadata)
            .expect_err("unknown session payload fields must fail closed");
        assert_eq!(err.reason(), REASON_AUTHORITY_FORMAT_INVALID);
        assert!(
            err.to_string()
                .contains("unknown field `retired_subject_locator`"),
            "{err}"
        );
    }

    #[test]
    fn session_authority_rejects_unknown_wire_fields() {
        let wire = serde_json::json!({
            "payload": session_payload(),
            "signature": "c2Vzc2lvbi1zaWduYXR1cmU=",
            "retired_signature_carrier": "compat-carrier"
        });
        let metadata = HashMap::from([(
            SESSION_AUTHORITY_METADATA_KEY.to_string(),
            encode_authority_wire(wire),
        )]);

        let err = project_admitted_session_authority(&metadata)
            .expect_err("unknown session wire fields must fail closed");
        assert_eq!(err.reason(), REASON_AUTHORITY_FORMAT_INVALID);
        assert!(
            err.to_string()
                .contains("unknown field `retired_signature_carrier`"),
            "{err}"
        );
    }

    #[test]
    fn delegation_authority_rejects_unknown_payload_fields() {
        let mut payload =
            serde_json::to_value(delegation_payload()).expect("delegation payload serializes");
        payload
            .as_object_mut()
            .expect("delegation payload is object")
            .insert(
                "retired_scope_carrier".to_string(),
                serde_json::json!("compat-carrier"),
            );
        let wire = serde_json::json!({
            "payload": payload,
            "signature": "ZGVsZWdhdGlvbi1zaWduYXR1cmU="
        });
        let metadata = HashMap::from([(
            DELEGATION_METADATA_KEY.to_string(),
            encode_authority_wire(wire),
        )]);

        let err = project_invocation_authority_metadata_shape(&metadata)
            .expect_err("unknown delegation payload fields must fail closed");
        assert_eq!(err.reason(), REASON_AUTHORITY_FORMAT_INVALID);
        assert!(
            err.to_string()
                .contains("unknown field `retired_scope_carrier`"),
            "{err}"
        );
    }

    #[test]
    fn delegation_authority_rejects_unknown_wire_fields() {
        let wire = serde_json::json!({
            "payload": delegation_payload(),
            "signature": "ZGVsZWdhdGlvbi1zaWduYXR1cmU=",
            "retired_signature_carrier": "compat-carrier"
        });
        let metadata = HashMap::from([(
            DELEGATION_METADATA_KEY.to_string(),
            encode_authority_wire(wire),
        )]);

        let err = project_invocation_authority_metadata_shape(&metadata)
            .expect_err("unknown delegation wire fields must fail closed");
        assert_eq!(err.reason(), REASON_AUTHORITY_FORMAT_INVALID);
        assert!(
            err.to_string()
                .contains("unknown field `retired_signature_carrier`"),
            "{err}"
        );
    }

    #[test]
    fn authority_clock_failure_is_not_projected_to_epoch_zero() {
        let err = unix_epoch_millis(UNIX_EPOCH - Duration::from_millis(1)).unwrap_err();
        assert_eq!(err.reason(), REASON_AUTHORITY_CLOCK_UNAVAILABLE);
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
    fn session_authority_rejects_retired_invocation_history_subject_carrier() {
        let mut payload = session_payload();
        payload.session_id = "invocation_history".into();
        payload.subject_ura =
            "easynet:///r/example/resource/user.alice/session/invocation_history".into();

        let err = validate_session_authority_payload_shape(&payload, None)
            .expect_err("retired invocation-history subject carrier must fail closed");
        assert_eq!(err.reason(), REASON_AUTHORITY_FORMAT_INVALID);
        assert!(
            err.to_string()
                .contains("retired invocation-history subject"),
            "{err}"
        );
        assert!(
            !session_authority_admits_subject(&payload, &payload.subject_ura),
            "exact-match admission must not revive the retired carrier"
        );
    }

    #[test]
    fn session_authority_rejects_all_zero_owner_before_subject_admission() {
        let all_zero = "00000000-0000-0000-0000-000000000000";
        let mut payload = session_payload();
        payload.session_owner_user_id = all_zero.into();
        payload.subject_ura = crate::core::ura::resource_dot_ura(
            "example",
            &format!("user.{all_zero}"),
            "session/session-1",
        );

        let err = validate_session_authority_payload_shape(&payload, None)
            .expect_err("all-zero session owner must fail at authority metadata validation");
        assert_eq!(err.reason(), REASON_AUTHORITY_FORMAT_INVALID);
        assert!(
            err.to_string()
                .contains("session_owner_user_id must not be all-zero"),
            "{err}"
        );
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
            authority_subject_kind(
                "easynet:///r/example/resource/user.alice/session/invocation_history"
            ),
            AuthoritySubjectKind::Other,
            "retired invocation-history carrier must not classify as a live session subject"
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
