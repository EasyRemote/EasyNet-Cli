// EasyNet CLI — invocation authority metadata core
// =================================================
//
// File: src/daemon/invocation/admission/authority_metadata.rs
// Description: Daemon SDK core projection for delegated and session authority
//              signing material plus wire metadata materialization.
//
// Protocol Responsibility
// -----------------------
// Own the EasyNet-Cli side of daemon admission authority metadata without
// moving Axon semantics into Go/Python facades. The daemon still verifies
// signatures and trust anchors at admission time; this module only prepares the
// exact bytes that are signed and wraps a caller-provided signature into the
// metadata shape admission already accepts.
//
// Implementation Approach
// -----------------------
// Authority payloads are typed Rust domain objects. Canonical bytes are derived
// through the daemon's canonical JSON helper, matching admission verification.
// Signing is intentionally outside this module so private keys never cross the
// C ABI.
//
// Usage Contract
// --------------
// Callers pass request JSON, receive signing material, sign
// `canonical_bytes_base64`, and pass only signature bytes/base64 back to
// materialize metadata. Request metadata carrying private key material is
// rejected at this boundary.
//
// Architectural Position
// ----------------------
// This module is the lower-layer SDK core used by both daemon admission and
// `libeasynet_cli` authority ABI projection. Language SDKs are facades over
// this contract, not owners of canonical authority payload construction.

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
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
struct DelegationRequest {
    issuer_ura: String,
    subject_ura: String,
    caller_ura: String,
    audience: String,
    scopes: Vec<String>,
    issued_at_ms: i64,
    expires_at_ms: i64,
    #[serde(default)]
    metadata: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
struct SessionAuthorityRequest {
    issuer_ura: String,
    session_id: String,
    session_owner_user_id: String,
    creator_principal_id: String,
    callee_ura: String,
    subject_ura: String,
    audience: String,
    scopes: Vec<String>,
    allowed_actions: Vec<String>,
    allowed_followup_abilities: Vec<String>,
    issued_at_ms: i64,
    expires_at_ms: i64,
    #[serde(default)]
    metadata: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
struct AuthoritySignature {
    signature_base64: String,
}

#[derive(Debug, Serialize)]
struct SignedAuthorityWire<T> {
    payload: T,
    signature: String,
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

pub(crate) fn prepare_delegation_from_json(
    request_json: &str,
) -> Result<Value, AuthorityMetadataError> {
    let request = parse_delegation_request(request_json)?;
    let payload = request.into_payload()?;
    signing_material("delegation", DELEGATION_METADATA_KEY, &payload)
}

pub(crate) fn materialize_delegation_from_json(
    request_json: &str,
    signature_json: &str,
) -> Result<Value, AuthorityMetadataError> {
    let request = parse_delegation_request(request_json)?;
    let payload = request.into_payload()?;
    let signature_base64 = parse_signature(signature_json)?;
    materialize_metadata(
        "delegation",
        DELEGATION_METADATA_KEY,
        payload,
        signature_base64,
    )
}

pub(crate) fn prepare_session_authority_from_json(
    request_json: &str,
) -> Result<Value, AuthorityMetadataError> {
    let request = parse_session_authority_request(request_json)?;
    let payload = request.into_payload()?;
    signing_material(
        "session_authority",
        SESSION_AUTHORITY_METADATA_KEY,
        &payload,
    )
}

pub(crate) fn materialize_session_authority_from_json(
    request_json: &str,
    signature_json: &str,
) -> Result<Value, AuthorityMetadataError> {
    let request = parse_session_authority_request(request_json)?;
    let payload = request.into_payload()?;
    let signature_base64 = parse_signature(signature_json)?;
    materialize_metadata(
        "session_authority",
        SESSION_AUTHORITY_METADATA_KEY,
        payload,
        signature_base64,
    )
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

impl DelegationRequest {
    fn into_payload(self) -> Result<DelegationPayload, AuthorityMetadataError> {
        reject_private_key_metadata(&self.metadata)?;
        let payload = DelegationPayload {
            issuer_ura: self.issuer_ura,
            subject_ura: self.subject_ura,
            caller_ura: self.caller_ura,
            audience: self.audience,
            scopes: self.scopes,
            issued_at_ms: self.issued_at_ms,
            expires_at_ms: self.expires_at_ms,
        };
        validate_delegation_payload_shape(&payload, None)?;
        Ok(payload)
    }
}

impl SessionAuthorityRequest {
    fn into_payload(self) -> Result<SessionAuthorityPayload, AuthorityMetadataError> {
        reject_private_key_metadata(&self.metadata)?;
        let payload = SessionAuthorityPayload {
            issuer_ura: self.issuer_ura,
            session_id: self.session_id,
            session_owner_user_id: self.session_owner_user_id,
            creator_principal_id: self.creator_principal_id,
            callee_ura: self.callee_ura,
            subject_ura: self.subject_ura,
            audience: self.audience,
            scopes: self.scopes,
            allowed_actions: self.allowed_actions,
            allowed_followup_abilities: self.allowed_followup_abilities,
            issued_at_ms: self.issued_at_ms,
            expires_at_ms: self.expires_at_ms,
        };
        validate_session_authority_payload_shape(&payload, None)?;
        Ok(payload)
    }
}

fn parse_delegation_request(raw: &str) -> Result<DelegationRequest, AuthorityMetadataError> {
    serde_json::from_str(raw).map_err(|err| {
        AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            format!("delegation request JSON parse failed: {err}"),
        )
    })
}

fn parse_session_authority_request(
    raw: &str,
) -> Result<SessionAuthorityRequest, AuthorityMetadataError> {
    serde_json::from_str(raw).map_err(|err| {
        AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            format!("session authority request JSON parse failed: {err}"),
        )
    })
}

fn parse_signature(raw: &str) -> Result<String, AuthorityMetadataError> {
    let signature: AuthoritySignature = serde_json::from_str(raw).map_err(|err| {
        AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            format!("authority signature JSON parse failed: {err}"),
        )
    })?;
    let signature_base64 = signature.signature_base64.trim();
    if signature_base64.is_empty() {
        return Err(AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            "authority signature_base64 is required",
        ));
    }
    BASE64_STANDARD.decode(signature_base64).map_err(|err| {
        AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            format!("authority signature base64 decode failed: {err}"),
        )
    })?;
    Ok(signature_base64.to_string())
}

fn signing_material<T: Serialize>(
    kind: &'static str,
    metadata_key: &'static str,
    payload: &T,
) -> Result<Value, AuthorityMetadataError> {
    let canonical = canonical_authority_payload_bytes(payload)?;
    let canonical_hash_hex = Sha256::digest(&canonical)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(json!({
        "profile": "authority",
        "kind": kind,
        "algorithm": "ed25519",
        "metadata_key": metadata_key,
        "canonical_bytes_base64": BASE64_STANDARD.encode(&canonical),
        "canonical_hash_hex": canonical_hash_hex,
        "signed_fields": authority_signed_fields(kind),
        "payload": payload,
    }))
}

fn materialize_metadata<T: Serialize>(
    kind: &'static str,
    metadata_key: &'static str,
    payload: T,
    signature_base64: String,
) -> Result<Value, AuthorityMetadataError> {
    let wire = SignedAuthorityWire {
        payload,
        signature: signature_base64,
    };
    let wire_json = serde_json::to_vec(&wire).map_err(|err| {
        AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            format!("authority metadata wire marshal failed: {err}"),
        )
    })?;
    let metadata_value = BASE64_STANDARD.encode(wire_json);
    Ok(json!({
        "profile": "authority",
        "kind": kind,
        "metadata_key": metadata_key,
        "metadata_value": metadata_value,
        "metadata": {
            metadata_key: metadata_value,
        },
    }))
}

fn authority_signed_fields(kind: &str) -> &'static [&'static str] {
    match kind {
        "delegation" => &[
            "issuer_ura",
            "subject_ura",
            "caller_ura",
            "audience",
            "scopes",
            "issued_at_ms",
            "expires_at_ms",
        ],
        "session_authority" => &[
            "issuer_ura",
            "session_id",
            "session_owner_user_id",
            "creator_principal_id",
            "callee_ura",
            "subject_ura",
            "audience",
            "scopes",
            "allowed_actions",
            "allowed_followup_abilities",
            "issued_at_ms",
            "expires_at_ms",
        ],
        _ => &[],
    }
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

fn reject_private_key_metadata(
    metadata: &Map<String, Value>,
) -> Result<(), AuthorityMetadataError> {
    reject_private_key_metadata_value(&Value::Object(metadata.clone()))
}

fn reject_private_key_metadata_value(value: &Value) -> Result<(), AuthorityMetadataError> {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                reject_private_key_metadata_key(key)?;
                reject_private_key_metadata_value(child)?;
            }
            Ok(())
        }
        Value::Array(items) => {
            for child in items {
                reject_private_key_metadata_value(child)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn reject_private_key_metadata_key(key: &str) -> Result<(), AuthorityMetadataError> {
    let normalized = key.to_ascii_lowercase().replace(['-', '_'], "");
    if normalized.contains("privatekey")
        || normalized.contains("secretkey")
        || normalized == "seed"
        || normalized.contains("signingseed")
    {
        return Err(AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            format!("authority request metadata must not carry private key material: `{key}`"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DELEGATION_REQUEST: &str = r#"{
      "issuer_ura":"easynet:///r/example/user/alice",
      "subject_ura":"easynet:///r/example/user/alice",
      "caller_ura":"easynet:///r/example/agent/backend",
      "audience":"easynet:///r/example/device/dev-a",
      "scopes":["device.observe.*"],
      "issued_at_ms":1000,
      "expires_at_ms":2000
    }"#;

    const SESSION_REQUEST: &str = r#"{
      "issuer_ura":"easynet:///r/example/agent/backend",
      "session_id":"session-1",
      "session_owner_user_id":"alice",
      "creator_principal_id":"easynet:///r/example/agent/backend",
      "callee_ura":"easynet:///r/example/device/dev-a",
      "subject_ura":"easynet:///r/example/resource/user.alice/session/session-1",
      "audience":"easynet:///r/example/device/dev-a",
      "scopes":["device.observe.*"],
      "allowed_actions":["read"],
      "allowed_followup_abilities":["device.observe.health"],
      "issued_at_ms":1000,
      "expires_at_ms":2000
    }"#;

    #[test]
    fn prepare_delegation_returns_canonical_material() {
        let material = prepare_delegation_from_json(DELEGATION_REQUEST).unwrap();
        assert_eq!(material["profile"], "authority");
        assert_eq!(material["kind"], "delegation");
        assert_eq!(material["algorithm"], "ed25519");
        assert_eq!(material["metadata_key"], DELEGATION_METADATA_KEY);
        assert!(material["canonical_hash_hex"].as_str().unwrap().len() == 64);
        let canonical = BASE64_STANDARD
            .decode(material["canonical_bytes_base64"].as_str().unwrap())
            .unwrap();
        assert_eq!(
            String::from_utf8(canonical).unwrap(),
            r#"{"audience":"easynet:///r/example/device/dev-a","caller_ura":"easynet:///r/example/agent/backend","expires_at_ms":2000,"issued_at_ms":1000,"issuer_ura":"easynet:///r/example/user/alice","scopes":["device.observe.*"],"subject_ura":"easynet:///r/example/user/alice"}"#
        );
    }

    #[test]
    fn materialize_session_authority_returns_admission_metadata_shape() {
        let projection = materialize_session_authority_from_json(
            SESSION_REQUEST,
            r#"{"signature_base64":"c2Vzc2lvbi1zaWduYXR1cmU="}"#,
        )
        .unwrap();
        let value = projection["metadata"][SESSION_AUTHORITY_METADATA_KEY]
            .as_str()
            .unwrap();
        let wire = BASE64_STANDARD.decode(value).unwrap();
        let wire_json: Value = serde_json::from_slice(&wire).unwrap();
        assert_eq!(
            wire_json["payload"]["issuer_ura"],
            "easynet:///r/example/agent/backend"
        );
        assert_eq!(
            wire_json["payload"]["subject_ura"],
            "easynet:///r/example/resource/user.alice/session/session-1"
        );
        assert_eq!(wire_json["payload"]["session_id"], "session-1");
        assert_eq!(wire_json["payload"]["allowed_actions"][0], "read");
        assert_eq!(wire_json["signature"], "c2Vzc2lvbi1zaWduYXR1cmU=");
    }

    #[test]
    fn session_authority_requires_followup_binding_fields() {
        let err = prepare_session_authority_from_json(
            r#"{
              "issuer_ura":"easynet:///r/example/agent/backend",
              "subject_ura":"easynet:///r/example/resource/user.alice/session/session-1",
              "audience":"easynet:///r/example/device/dev-a",
              "scopes":["device.observe.*"],
              "issued_at_ms":1000,
              "expires_at_ms":2000
            }"#,
        )
        .unwrap_err();
        assert_eq!(err.reason(), REASON_AUTHORITY_FORMAT_INVALID);
    }

    #[test]
    fn session_authority_binds_subject_resource_to_declared_owner_and_session() {
        for request in [
            SESSION_REQUEST.replace("resource/user.alice", "resource/user.bob"),
            SESSION_REQUEST.replace("session/session-1", "session/session-2"),
            SESSION_REQUEST.replace("resource/user.alice/session/session-1", "session/session-1"),
        ] {
            let err = prepare_session_authority_from_json(&request).unwrap_err();
            assert_eq!(err.reason(), REASON_AUTHORITY_FORMAT_INVALID);
        }

        let mismatched_user_subject =
            SESSION_REQUEST.replace("resource/user.alice/session/session-1", "user/bob");
        let err = prepare_session_authority_from_json(&mismatched_user_subject).unwrap_err();
        assert_eq!(err.reason(), REASON_AUTHORITY_FORMAT_INVALID);

        let matching_user_subject =
            SESSION_REQUEST.replace("resource/user.alice/session/session-1", "user/alice");
        prepare_session_authority_from_json(&matching_user_subject)
            .expect("the declared session owner remains a canonical user subject");
    }

    #[test]
    fn request_metadata_rejects_private_key_material() {
        let err = prepare_delegation_from_json(
            r#"{
              "issuer_ura":"easynet:///r/example/user/alice",
              "subject_ura":"easynet:///r/example/user/alice",
              "caller_ura":"easynet:///r/example/agent/backend",
              "audience":"easynet:///r/example/device/dev-a",
              "scopes":["device.observe.*"],
              "issued_at_ms":1000,
              "expires_at_ms":2000,
              "metadata":{"nested":{"secret_key":"never"}}
            }"#,
        )
        .unwrap_err();
        assert_eq!(err.reason(), REASON_AUTHORITY_FORMAT_INVALID);
    }

    #[test]
    fn materialize_rejects_legacy_signature_alias() {
        let err = materialize_delegation_from_json(
            DELEGATION_REQUEST,
            r#"{"signature":"ZGVsZWdhdGlvbi1zaWduYXR1cmU="}"#,
        )
        .unwrap_err();
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
