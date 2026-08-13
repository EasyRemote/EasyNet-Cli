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

pub(crate) const DELEGATION_METADATA_KEY: &str =
    crate::daemon::ability::RUNTIME_DELEGATION_METADATA_KEY;
pub(crate) const SESSION_AUTHORITY_METADATA_KEY: &str = "x-runtime-session-authority";
pub(crate) const REASON_AUTHORITY_FORMAT_INVALID: &str = "AUTHORITY_FORMAT_INVALID";
pub(crate) const REASON_AUTHORITY_EXPIRED: &str = "AUTHORITY_EXPIRED";
pub(crate) const REASON_AUTHORITY_CLOCK_UNAVAILABLE: &str = "AUTHORITY_CLOCK_UNAVAILABLE";
pub(crate) const REASON_AUTHORITY_SIGNING_FAILED: &str = "AUTHORITY_SIGNING_FAILED";

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

pub(crate) type DelegationPayload = crate::daemon::ability::DelegationAuthorityClaims;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionAuthorityPayload {
    pub(crate) issuer_ura: String,
    pub(crate) session_id: String,
    /// Public session-authority wire scalar: the User id segment bound into
    /// `subject_ura`. It is not a User URA; admission converts it to a canonical
    /// User URA only at issuer-policy comparison boundaries.
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

/// Generic request for one canonical session-authority metadata value.
///
/// This is the Rust implementation of the same runtime model exposed by the
/// Go/Python SDKs. Product code supplies only runtime facts; key custody is an
/// injected signing capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionAuthorityRequest {
    pub(crate) issuer_ura: String,
    pub(crate) session_id: String,
    /// Public session-authority request scalar. Keep the wire key
    /// `session_owner_user_id`; do not use this field as a runtime User URA.
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

impl From<SessionAuthorityRequest> for SessionAuthorityPayload {
    fn from(request: SessionAuthorityRequest) -> Self {
        Self {
            issuer_ura: request.issuer_ura,
            session_id: request.session_id,
            session_owner_user_id: request.session_owner_user_id,
            creator_principal_id: request.creator_principal_id,
            callee_ura: request.callee_ura,
            subject_ura: request.subject_ura,
            audience: request.audience,
            scopes: request.scopes,
            allowed_actions: request.allowed_actions,
            allowed_followup_abilities: request.allowed_followup_abilities,
            issued_at_ms: request.issued_at_ms,
            expires_at_ms: request.expires_at_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IssuedAuthorityMetadata {
    key: &'static str,
    value: String,
}

impl IssuedAuthorityMetadata {
    pub(crate) fn key(&self) -> &'static str {
        self.key
    }

    pub(crate) fn value(&self) -> &str {
        &self.value
    }

    pub(crate) fn into_map(self) -> HashMap<String, String> {
        HashMap::from([(self.key.to_string(), self.value)])
    }
}

/// SDK-aligned canonical authority provider.
///
/// The provider owns validation, canonical bytes, and wire projection. The
/// supplied closure is the only key-custody seam and must already be bound to
/// `signer_ura`; raw private key material never crosses this API.
pub(crate) struct CanonicalSessionAuthorityIssuer;

impl CanonicalSessionAuthorityIssuer {
    pub(crate) fn prepare(
        request: SessionAuthorityRequest,
        signer_ura: &str,
    ) -> Result<PreparedSessionAuthority, AuthorityMetadataError> {
        let payload = SessionAuthorityPayload::from(request);
        let signer_ura = signer_ura.trim();
        if signer_ura.is_empty() || payload.issuer_ura != signer_ura {
            return Err(AuthorityMetadataError::new(
                REASON_AUTHORITY_FORMAT_INVALID,
                "session authority issuer must match the bound signer owner",
            ));
        }
        validate_session_authority_payload_shape(&payload, None)?;
        let canonical_payload = canonical_authority_payload_bytes(&payload)?;
        Ok(PreparedSessionAuthority {
            payload,
            canonical_payload,
        })
    }

    pub(crate) fn issue<E>(
        request: SessionAuthorityRequest,
        signer_ura: &str,
        sign: impl FnOnce(&[u8]) -> Result<Vec<u8>, E>,
    ) -> Result<IssuedAuthorityMetadata, AuthorityMetadataError>
    where
        E: std::fmt::Display,
    {
        let prepared = Self::prepare(request, signer_ura)?;
        let signature = sign(prepared.canonical_payload()).map_err(|error| {
            AuthorityMetadataError::new(
                REASON_AUTHORITY_SIGNING_FAILED,
                format!("session authority signer rejected canonical payload: {error}"),
            )
        })?;
        prepared.seal(signature)
    }
}

/// Validated, canonical authority payload awaiting an opaque signer.
///
/// Keeping this as an explicit state prevents async/remote signers from
/// rebuilding the payload after signature generation.
pub(crate) struct PreparedSessionAuthority {
    payload: SessionAuthorityPayload,
    canonical_payload: Vec<u8>,
}

impl PreparedSessionAuthority {
    pub(crate) fn canonical_payload(&self) -> &[u8] {
        &self.canonical_payload
    }

    pub(crate) fn seal(
        self,
        signature: Vec<u8>,
    ) -> Result<IssuedAuthorityMetadata, AuthorityMetadataError> {
        if signature.is_empty() {
            return Err(AuthorityMetadataError::new(
                REASON_AUTHORITY_SIGNING_FAILED,
                "session authority signer returned an empty signature",
            ));
        }
        let wire = serde_json::json!({
            "payload": self.payload,
            "signature": BASE64_STANDARD.encode(signature),
        });
        let wire_bytes = canonical_json_bytes(&wire);
        Ok(IssuedAuthorityMetadata {
            key: SESSION_AUTHORITY_METADATA_KEY,
            value: BASE64_STANDARD.encode(wire_bytes),
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SignedSessionAuthorityWire {
    pub(crate) payload: SessionAuthorityPayload,
    pub(crate) signature: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SignedDelegationAuthorityWire {
    pub(crate) payload: DelegationPayload,
    pub(crate) signature: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSessionAuthorityWire {
    payload: serde_json::Value,
    signature: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDelegationAuthorityWire {
    payload: serde_json::Value,
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
    let wire = decode_delegation_authority_wire(raw)?;
    let now_ms = current_unix_epoch_millis()?;
    validate_delegation_payload_shape(&wire.payload, Some(now_ms))?;
    Ok(wire.payload)
}

pub(crate) fn decode_delegation_authority_wire(
    raw: &str,
) -> Result<SignedDelegationAuthorityWire, AuthorityMetadataError> {
    let wire_bytes = BASE64_STANDARD.decode(raw).map_err(|err| {
        AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            format!("delegation authority base64 decode failed: {err}"),
        )
    })?;
    let raw_wire: RawDelegationAuthorityWire =
        serde_json::from_slice(&wire_bytes).map_err(|err| {
            AuthorityMetadataError::new(
                REASON_AUTHORITY_FORMAT_INVALID,
                format!("delegation authority JSON parse failed: {err}"),
            )
        })?;
    if raw_wire.signature.trim().is_empty() {
        return Err(AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            "delegation authority signature is empty",
        ));
    }
    let payload: DelegationPayload = serde_json::from_value(raw_wire.payload).map_err(|err| {
        AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            format!("delegation authority payload parse failed: {err}"),
        )
    })?;
    Ok(SignedDelegationAuthorityWire {
        payload,
        signature: raw_wire.signature,
    })
}

fn project_session_authority_shape(
    raw: &str,
) -> Result<SessionAuthorityPayload, AuthorityMetadataError> {
    let wire = decode_session_authority_wire(raw)?;
    let now_ms = current_unix_epoch_millis()?;
    validate_session_authority_payload_shape(&wire.payload, Some(now_ms))?;
    Ok(wire.payload)
}

pub(crate) fn decode_session_authority_wire(
    raw: &str,
) -> Result<SignedSessionAuthorityWire, AuthorityMetadataError> {
    let wire_bytes = BASE64_STANDARD.decode(raw).map_err(|err| {
        AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            format!("session authority base64 decode failed: {err}"),
        )
    })?;
    let raw_wire: RawSessionAuthorityWire = serde_json::from_slice(&wire_bytes).map_err(|err| {
        AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            format!("session authority JSON parse failed: {err}"),
        )
    })?;
    if raw_wire.signature.trim().is_empty() {
        return Err(AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            "session authority signature is empty",
        ));
    }
    let payload: SessionAuthorityPayload =
        serde_json::from_value(raw_wire.payload).map_err(|err| {
            AuthorityMetadataError::new(
                REASON_AUTHORITY_FORMAT_INVALID,
                format!("session authority payload parse failed: {err}"),
            )
        })?;
    Ok(SignedSessionAuthorityWire {
        payload,
        signature: raw_wire.signature,
    })
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
    validate_callable_authority_target("authority audience", &payload.audience)?;
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
    if !crate::core::identity::is_canonical_session_authority_id(&payload.session_id) {
        return Err(AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            "session authority session_id is not canonical",
        ));
    }
    validate_callable_authority_target("session authority callee_ura", &payload.callee_ura)?;
    validate_callable_authority_target("session authority audience", &payload.audience)?;
    let subject_kind = authority_subject_kind(&payload.subject_ura);
    if !matches!(
        subject_kind,
        AuthoritySubjectKind::User
            | AuthoritySubjectKind::Agent
            | AuthoritySubjectKind::Session
            | AuthoritySubjectKind::DescriptorBound
            | AuthoritySubjectKind::RuntimeStateRead
            | AuthoritySubjectKind::Resource
    ) {
        return Err(AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            format!(
                "session authority subject_ura `{}` must be a canonical User, Agent, or Resource authority subject",
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
    } else if subject_kind == AuthoritySubjectKind::DescriptorBound {
        let parsed = crate::core::ura::parse_ura(payload.subject_ura.trim()).map_err(|err| {
            AuthorityMetadataError::new(
                REASON_AUTHORITY_FORMAT_INVALID,
                format!("session authority subject_ura parse failed: {err}"),
            )
        })?;
        let (owner_user_id, _operation, ability) =
            canonical_descriptor_bound_resource_parts(&parsed).ok_or_else(|| {
                AuthorityMetadataError::new(
                    REASON_AUTHORITY_FORMAT_INVALID,
                    "session authority subject_ura must name one canonical descriptor-bound user or service resource",
                )
            })?;
        if owner_user_id != payload.session_owner_user_id {
            return Err(AuthorityMetadataError::new(
                REASON_AUTHORITY_FORMAT_INVALID,
                format!(
                    "session authority descriptor-bound subject owner must match session_owner_user_id `{}`",
                    payload.session_owner_user_id
                ),
            ));
        }
        if !payload
            .allowed_followup_abilities
            .iter()
            .any(|allowed| allowed.trim() == ability)
        {
            return Err(AuthorityMetadataError::new(
                REASON_AUTHORITY_FORMAT_INVALID,
                "session authority descriptor-bound subject ability must be an exact allowed follow-up ability",
            ));
        }
    } else if subject_kind == AuthoritySubjectKind::RuntimeStateRead {
        let parsed = crate::core::ura::parse_ura(payload.subject_ura.trim()).map_err(|err| {
            AuthorityMetadataError::new(
                REASON_AUTHORITY_FORMAT_INVALID,
                format!("session authority subject_ura parse failed: {err}"),
            )
        })?;
        let owner_user_id = canonical_runtime_state_read_resource_owner(&parsed).ok_or_else(|| {
            AuthorityMetadataError::new(
                REASON_AUTHORITY_FORMAT_INVALID,
                "session authority subject_ura must name one canonical user runtime-state read resource",
            )
        })?;
        if owner_user_id != payload.session_owner_user_id {
            return Err(AuthorityMetadataError::new(
                REASON_AUTHORITY_FORMAT_INVALID,
                format!(
                    "session authority runtime-state read subject owner must match session_owner_user_id `{}`",
                    payload.session_owner_user_id
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

/// Authority metadata may target only principals that own callable ability
/// surfaces. A Device is transport/key custody, not an invocation callee; a
/// wildcard or resource-prefix audience would erase that ontology at the wire
/// boundary and is therefore rejected before signature or policy evaluation.
fn validate_callable_authority_target(
    label: &str,
    target_ura: &str,
) -> Result<(), AuthorityMetadataError> {
    let parsed = crate::core::ura::parse_ura(target_ura.trim()).map_err(|err| {
        AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            format!("{label} must be a canonical Agent, Service, or Authority URA: {err}"),
        )
    })?;
    if !matches!(
        parsed.kind,
        crate::core::ura::URAKind::Agent
            | crate::core::ura::URAKind::Service
            | crate::core::ura::URAKind::Authority
    ) {
        return Err(AuthorityMetadataError::new(
            REASON_AUTHORITY_FORMAT_INVALID,
            format!("{label} must identify a callable Agent, Service, or Authority principal"),
        ));
    }
    Ok(())
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
    let Ok(parsed) = crate::core::ura::parse_ura(subject_ura.trim()) else {
        return AuthoritySubjectKind::Other;
    };
    match parsed.kind {
        crate::core::ura::URAKind::User => AuthoritySubjectKind::User,
        crate::core::ura::URAKind::Service => AuthoritySubjectKind::Service,
        crate::core::ura::URAKind::Agent => AuthoritySubjectKind::Agent,
        crate::core::ura::URAKind::Resource
            if canonical_session_resource_parts(&parsed).is_some() =>
        {
            AuthoritySubjectKind::Session
        }
        crate::core::ura::URAKind::Resource
            if canonical_descriptor_bound_resource_parts(&parsed).is_some() =>
        {
            AuthoritySubjectKind::DescriptorBound
        }
        crate::core::ura::URAKind::Resource
            if canonical_runtime_state_read_resource_owner(&parsed).is_some() =>
        {
            AuthoritySubjectKind::RuntimeStateRead
        }
        crate::core::ura::URAKind::Resource
            if parsed
                .resource_owner_id()
                .is_some_and(|owner| !owner.starts_with("user.")) =>
        {
            AuthoritySubjectKind::Resource
        }
        _ => AuthoritySubjectKind::Other,
    }
}

fn canonical_descriptor_bound_resource_parts(
    parsed: &crate::core::ura::ParsedURA,
) -> Option<(&str, &str, &str)> {
    let owner_user_id = descriptor_bound_resource_accountable_user_id(parsed)?;
    let (operation, ability) = parsed.resource_path()?.split_once('/')?;
    if !matches!(operation, "read" | "invoke" | "stream" | "manage" | "grant") {
        return None;
    }
    if ability.is_empty() || ability.contains('/') {
        return None;
    }
    Some((owner_user_id, operation, ability))
}

fn descriptor_bound_resource_accountable_user_id(
    parsed: &crate::core::ura::ParsedURA,
) -> Option<&str> {
    let owner = parsed.resource_owner_id()?;
    if let Some(owner_user_id) = owner.strip_prefix("user.") {
        return (!owner_user_id.is_empty() && !owner_user_id.contains('.'))
            .then_some(owner_user_id);
    }
    let service_owner = owner.strip_prefix("service.")?;
    let (principal_id, service_id) = service_owner.split_once('.')?;
    if principal_id.is_empty()
        || principal_id.contains('.')
        || service_id.is_empty()
        || service_id.contains('.')
    {
        return None;
    }
    Some(principal_id)
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
        .filter(|session_id| {
            crate::core::identity::is_canonical_session_authority_id(session_id)
        })?;
    Some((owner_user_id, session_id))
}

/// Project the identity carried by one canonical User-owned session subject.
///
/// Authority issuers use this instead of re-parsing `resource/user.<id>/session/<id>`
/// outside the admission model. Returning owned values keeps the parser's
/// internal representation private while preserving one source of truth for
/// the owner/session binding rule.
pub(crate) fn canonical_user_session_subject_identity(
    subject_ura: &str,
) -> Option<(String, String)> {
    let parsed = crate::core::ura::parse_ura(subject_ura.trim()).ok()?;
    let (owner_user_id, session_id) = canonical_session_resource_parts(&parsed)?;
    Some((owner_user_id.to_string(), session_id.to_string()))
}

fn canonical_runtime_state_read_resource_owner(
    parsed: &crate::core::ura::ParsedURA,
) -> Option<&str> {
    let owner_user_id = parsed
        .resource_owner_id()
        .and_then(|owner| owner.strip_prefix("user."))?;
    if owner_user_id.is_empty() || owner_user_id.contains('.') {
        return None;
    }
    (parsed.resource_path() == Some("runtime-state/read")).then_some(owner_user_id)
}

pub(crate) fn session_authority_admits_subject(
    payload: &SessionAuthorityPayload,
    subject: &str,
) -> bool {
    if !crate::core::identity::is_canonical_session_authority_id(&payload.session_id) {
        return false;
    }
    payload.subject_ura == subject
}

pub(crate) fn authority_audience_admits(audience: &str, callee: &str) -> bool {
    let audience = audience.trim();
    let callee = callee.trim();
    audience == "*" || audience == callee || audience.ends_with('/') && callee.starts_with(audience)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthoritySubjectKind {
    User,
    Service,
    Agent,
    Session,
    DescriptorBound,
    RuntimeStateRead,
    Resource,
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
            issuer_ura: "easynet:///r/example/agent/alice.backend".into(),
            session_id: "session-1".into(),
            session_owner_user_id: "alice".into(),
            creator_principal_id: "easynet:///r/example/agent/alice.backend".into(),
            callee_ura: "easynet:///r/example/agent/alice.backend".into(),
            subject_ura: "easynet:///r/example/resource/user.alice/session/session-1".into(),
            audience: "easynet:///r/example/agent/alice.backend".into(),
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
            caller_ura: "easynet:///r/example/agent/alice.backend".into(),
            audience: "easynet:///r/example/agent/alice.backend".into(),
            scopes: vec!["device.observe.*".into()],
            issued_at_ms: 1000,
            expires_at_ms: 4_102_444_800_000,
        }
    }

    fn encode_authority_wire(wire: serde_json::Value) -> String {
        BASE64_STANDARD.encode(serde_json::to_vec(&wire).expect("authority wire serializes"))
    }

    #[test]
    fn canonical_session_authority_issuer_matches_sdk_wire_contract() {
        let payload = session_payload();
        let expected = payload.clone();
        let signer_ura = payload.issuer_ura.clone();
        let issued = CanonicalSessionAuthorityIssuer::issue(
            SessionAuthorityRequest {
                issuer_ura: payload.issuer_ura,
                session_id: payload.session_id,
                session_owner_user_id: payload.session_owner_user_id,
                creator_principal_id: payload.creator_principal_id,
                callee_ura: payload.callee_ura,
                subject_ura: payload.subject_ura,
                audience: payload.audience,
                scopes: payload.scopes,
                allowed_actions: payload.allowed_actions,
                allowed_followup_abilities: payload.allowed_followup_abilities,
                issued_at_ms: payload.issued_at_ms,
                expires_at_ms: payload.expires_at_ms,
            },
            &signer_ura,
            |canonical| {
                assert_eq!(
                    canonical_authority_payload_bytes(&expected).unwrap(),
                    canonical
                );
                Ok::<_, std::convert::Infallible>(vec![0x5a; 64])
            },
        )
        .expect("issue canonical session authority");

        assert_eq!(issued.key(), SESSION_AUTHORITY_METADATA_KEY);
        let decoded = decode_session_authority_wire(issued.value()).expect("decode issued wire");
        assert_eq!(decoded.payload, expected);
        assert_eq!(
            BASE64_STANDARD.decode(decoded.signature).unwrap(),
            vec![0x5a; 64]
        );
    }

    #[test]
    fn canonical_session_authority_issuer_rejects_signer_owner_mismatch() {
        let payload = session_payload();
        let error = CanonicalSessionAuthorityIssuer::issue(
            SessionAuthorityRequest {
                issuer_ura: payload.issuer_ura,
                session_id: payload.session_id,
                session_owner_user_id: payload.session_owner_user_id,
                creator_principal_id: payload.creator_principal_id,
                callee_ura: payload.callee_ura,
                subject_ura: payload.subject_ura,
                audience: payload.audience,
                scopes: payload.scopes,
                allowed_actions: payload.allowed_actions,
                allowed_followup_abilities: payload.allowed_followup_abilities,
                issued_at_ms: payload.issued_at_ms,
                expires_at_ms: payload.expires_at_ms,
            },
            "easynet:///r/example/agent/alice.other",
            |_| Ok::<_, std::convert::Infallible>(vec![0x5a; 64]),
        )
        .expect_err("issuer and signer owner must be identical");

        assert_eq!(error.reason(), REASON_AUTHORITY_FORMAT_INVALID);
        assert!(error.to_string().contains("bound signer owner"));
    }

    #[test]
    fn delegation_payload_has_one_canonical_signing_representation() {
        let mut payload = delegation_payload();
        payload.expires_at_ms = 2000;
        validate_delegation_payload_shape(&payload, None).unwrap();
        let canonical = canonical_authority_payload_bytes(&payload).unwrap();
        assert_eq!(
            String::from_utf8(canonical).unwrap(),
            r#"{"audience":"easynet:///r/example/agent/alice.backend","caller_ura":"easynet:///r/example/agent/alice.backend","expires_at_ms":2000,"issued_at_ms":1000,"issuer_ura":"easynet:///r/example/user/alice","scopes":["device.observe.*"],"subject_ura":"easynet:///r/example/user/alice"}"#
        );
    }

    #[test]
    fn delegation_authority_rejects_device_audience() {
        let mut payload = delegation_payload();
        payload.audience = "easynet:///r/example/device/dev-a".into();

        let err = validate_delegation_payload_shape(&payload, None)
            .expect_err("Device is key custody, not a callable authority audience");
        assert_eq!(err.reason(), REASON_AUTHORITY_FORMAT_INVALID);
        assert!(
            err.to_string()
                .contains("callable Agent, Service, or Authority"),
            "{err}"
        );
    }

    #[test]
    fn delegation_authority_accepts_service_audience() {
        let mut payload = delegation_payload();
        payload.audience = "easynet:///r/example/service/alice.pages".into();

        validate_delegation_payload_shape(&payload, None)
            .expect("Service is a callable authority audience");
    }

    #[test]
    fn delegation_payload_rejects_all_zero_principal_placeholders() {
        let payload = DelegationPayload {
            issuer_ura: "easynet:///r/example/user/alice".into(),
            subject_ura: "easynet:///r/example/user/00000000-0000-0000-0000-000000000000".into(),
            caller_ura: "easynet:///r/example/agent/alice.backend".into(),
            audience: "easynet:///r/example/agent/alice.backend".into(),
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

        let mut descriptor_bound = session_payload();
        descriptor_bound.subject_ura =
            "easynet:///r/example/resource/user.alice/invoke/principal.lifecycle.get".into();
        descriptor_bound.allowed_followup_abilities = vec!["principal.lifecycle.get".into()];
        validate_session_authority_payload_shape(&descriptor_bound, None)
            .expect("exact descriptor-bound user resource must be admitted");

        let mut runtime_state_read = session_payload();
        runtime_state_read.subject_ura =
            "easynet:///r/example/resource/user.alice/runtime-state/read".into();
        validate_session_authority_payload_shape(&runtime_state_read, None)
            .expect("exact user-owned runtime-state read resource must be admitted");
        runtime_state_read.subject_ura =
            "easynet:///r/example/resource/user.bob/runtime-state/read".into();
        validate_session_authority_payload_shape(&runtime_state_read, None)
            .expect_err("runtime-state read resource owner mismatch must fail closed");

        descriptor_bound.allowed_followup_abilities = vec!["principal.lifecycle.create".into()];
        let err = validate_session_authority_payload_shape(&descriptor_bound, None)
            .expect_err("descriptor-bound ability mismatch must fail closed");
        assert!(
            err.to_string().contains("exact allowed follow-up ability"),
            "{err}"
        );
    }

    #[test]
    fn session_authority_rejects_device_callee_and_audience() {
        let mut device_callee = session_payload();
        device_callee.callee_ura = "easynet:///r/example/device/dev-a".into();
        let err = validate_session_authority_payload_shape(&device_callee, None)
            .expect_err("Device must not enter SessionAuthority as a callee");
        assert_eq!(err.reason(), REASON_AUTHORITY_FORMAT_INVALID);
        assert!(err.to_string().contains("callee_ura"), "{err}");

        let mut device_audience = session_payload();
        device_audience.audience = "easynet:///r/example/device/dev-a".into();
        let err = validate_session_authority_payload_shape(&device_audience, None)
            .expect_err("Device must not enter SessionAuthority as an audience");
        assert_eq!(err.reason(), REASON_AUTHORITY_FORMAT_INVALID);
        assert!(err.to_string().contains("audience"), "{err}");
    }

    #[test]
    fn session_authority_accepts_service_callee_and_descriptor_bound_subject() {
        let mut payload = session_payload();
        payload.callee_ura = "easynet:///r/example/service/alice.pages".into();
        payload.audience = payload.callee_ura.clone();
        payload.subject_ura =
            "easynet:///r/example/resource/service.alice.pages/read/project_list".into();
        payload.allowed_actions = vec!["read".into()];
        payload.allowed_followup_abilities = vec!["project_list".into()];

        validate_session_authority_payload_shape(&payload, None)
            .expect("Service-owned descriptor-bound authority must be canonical");
    }

    #[test]
    fn session_authority_admits_only_exact_payload_subject() {
        let payload = session_payload();
        assert!(
            session_authority_admits_subject(&payload, &payload.subject_ura),
            "session authority must admit its exact canonical subject"
        );

        for subject_ura in [
            "easynet:///r/example/resource/user.alice/session/session-1/terminal/default",
            "easynet:///r/example/resource/user.alice/runtime-state/read",
            "easynet:///r/example/resource/agent.alice.backend/runtime-state/read",
            "easynet:///r/example/user/alice",
        ] {
            assert!(
                !session_authority_admits_subject(&payload, subject_ura),
                "session authority must not infer same-owner authority for {subject_ura}"
            );
        }
    }

    #[test]
    fn session_authority_rejects_noncanonical_session_subject_carrier() {
        let mut payload = session_payload();
        payload.session_id = "invocation_history".into();
        payload.subject_ura =
            "easynet:///r/example/resource/user.alice/session/invocation_history".into();

        let err = validate_session_authority_payload_shape(&payload, None)
            .expect_err("noncanonical session carrier must fail closed");
        assert_eq!(err.reason(), REASON_AUTHORITY_FORMAT_INVALID);
        assert!(
            err.to_string().contains("session_id is not canonical"),
            "{err}"
        );
        assert!(
            !session_authority_admits_subject(&payload, &payload.subject_ura),
            "exact-match admission must not revive the noncanonical carrier"
        );
    }

    #[test]
    fn session_authority_rejects_request_scoped_noncanonical_session_subject_carrier() {
        let mut payload = session_payload();
        payload.session_id = "invocation_history:invocation.history.list:req-1".into();
        payload.subject_ura =
            "easynet:///r/example/resource/user.alice/session/invocation_history:invocation.history.list:req-1".into();

        let err = validate_session_authority_payload_shape(&payload, None)
            .expect_err("request-scoped noncanonical session carrier must fail closed");
        assert_eq!(err.reason(), REASON_AUTHORITY_FORMAT_INVALID);
        assert!(
            err.to_string().contains("session_id is not canonical"),
            "{err}"
        );
        assert_eq!(
            authority_subject_kind(&payload.subject_ura),
            AuthoritySubjectKind::Other,
            "request-scoped noncanonical carrier must not classify as a live session"
        );
        assert!(
            !session_authority_admits_subject(&payload, &payload.subject_ura),
            "exact-match admission must not revive request-scoped noncanonical carriers"
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
    fn authority_subject_kind_accepts_canonical_user_session_and_descriptor_resources() {
        assert_eq!(
            authority_subject_kind("easynet:///r/example/user/alice"),
            AuthoritySubjectKind::User
        );
        assert_eq!(
            authority_subject_kind("easynet:///r/example/agent/device.node-a.browser"),
            AuthoritySubjectKind::Agent
        );
        assert_eq!(
            authority_subject_kind("easynet:///r/example/service/alice.pages"),
            AuthoritySubjectKind::Service
        );
        assert_eq!(
            authority_subject_kind("easynet:///r/example/resource/user.alice/session/session-1"),
            AuthoritySubjectKind::Session
        );
        assert_eq!(
            authority_subject_kind(
                "easynet:///r/example/resource/service.alice.pages/read/project_list"
            ),
            AuthoritySubjectKind::DescriptorBound
        );
        assert_eq!(
            canonical_user_session_subject_identity(
                "easynet:///r/example/resource/user.alice/session/session-1"
            ),
            Some(("alice".to_string(), "session-1".to_string()))
        );
        assert_eq!(
            authority_subject_kind(
                "easynet:///r/example/resource/user.alice/invoke/principal.lifecycle.get"
            ),
            AuthoritySubjectKind::DescriptorBound
        );
        assert_eq!(
            authority_subject_kind("easynet:///r/example/resource/user.alice/runtime-state/read"),
            AuthoritySubjectKind::RuntimeStateRead
        );
        assert_eq!(
            authority_subject_kind("easynet:///r/example/resource/device.node-a/browser/session-1"),
            AuthoritySubjectKind::Resource
        );
        assert_eq!(
            authority_subject_kind(
                "easynet:///r/example/resource/user.alice/session/invocation_history"
            ),
            AuthoritySubjectKind::Other,
            "noncanonical session carrier must not classify as a live session subject"
        );
        assert_eq!(
            authority_subject_kind(
                "easynet:///r/example/resource/user.alice/session/invocation_history:invocation.history.list:req-1"
            ),
            AuthoritySubjectKind::Other,
            "request-scoped noncanonical session carrier must not classify as a live session subject"
        );
        assert_eq!(
            authority_subject_kind("easynet:///r/example/session/session-1"),
            AuthoritySubjectKind::Other,
            "the Axon URA grammar has no top-level session role"
        );
        assert_eq!(
            authority_subject_kind("easynet:///r/example/resource/device.dev-a/session/session-1"),
            AuthoritySubjectKind::Resource,
            "a Device resource is canonical but is not a User session resource"
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
