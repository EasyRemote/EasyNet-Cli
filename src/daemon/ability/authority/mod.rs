// EasyNet CLI - AuthorityBinding registry facts
// =============================================
//
// File: src/daemon/ability/authority/mod.rs
// Description: Daemon-local governance predicates for ability advertisement
//              and invocation. Axon owns the proof envelope; EasyNet-Cli owns
//              the local policy source and binding decision.

use std::collections::BTreeMap;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::daemon::identity::self_identity::CanonicalSigner;

use super::descriptors::{
    canonical_json_bytes, is_valid_ability_name, is_valid_descriptor_version,
    AbilityControlPlaneKey, CallMode,
};
use super::owner_projection::OwnerProjection;
use super::AbilityControlPlaneError;

const DEFAULT_INVOKE_POLICY_REF: &str = "ability_access_policy";
pub const HOSTED_AGENT_DELEGATION_METADATA_KEY: &str = "x-easynet-hosted-agent-delegation";
pub const HOSTED_AGENT_DELEGATION_REQUEST_METADATA_KEY: &str =
    "x-easynet-hosted-agent-delegation-request";
pub const RUNTIME_DELEGATION_METADATA_KEY: &str = "x-runtime-delegation";

/// Canonical delegated-authority claims shared by the Rust issuer and runtime
/// admission decoder.
///
/// This is the Rust counterpart of the public Go/Python SDK delegation DTO.
/// It owns wire shape only; admission remains responsible for deciding
/// whether the issuer is allowed to delegate for the requested subject.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationAuthorityClaims {
    pub(crate) issuer_ura: String,
    pub(crate) subject_ura: String,
    pub(crate) caller_ura: String,
    pub(crate) audience: String,
    pub(crate) scopes: Vec<String>,
    pub(crate) issued_at_ms: i64,
    pub(crate) expires_at_ms: i64,
}

impl DelegationAuthorityClaims {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        issuer_ura: impl Into<String>,
        subject_ura: impl Into<String>,
        caller_ura: impl Into<String>,
        audience: impl Into<String>,
        scopes: impl IntoIterator<Item = impl Into<String>>,
        issued_at_ms: i64,
        expires_at_ms: i64,
    ) -> anyhow::Result<Self> {
        let claims = Self {
            issuer_ura: issuer_ura.into(),
            subject_ura: subject_ura.into(),
            caller_ura: caller_ura.into(),
            audience: audience.into(),
            scopes: scopes.into_iter().map(Into::into).collect(),
            issued_at_ms,
            expires_at_ms,
        };
        claims.validate_shape()?;
        Ok(claims)
    }

    pub fn issuer_ura(&self) -> &str {
        &self.issuer_ura
    }

    pub fn subject_ura(&self) -> &str {
        &self.subject_ura
    }

    pub fn caller_ura(&self) -> &str {
        &self.caller_ura
    }

    pub fn audience(&self) -> &str {
        &self.audience
    }

    pub fn scopes(&self) -> &[String] {
        &self.scopes
    }

    pub fn issued_at_ms(&self) -> i64 {
        self.issued_at_ms
    }

    pub fn expires_at_ms(&self) -> i64 {
        self.expires_at_ms
    }

    pub fn canonical_payload_bytes(&self) -> Vec<u8> {
        canonical_json_bytes(
            &serde_json::to_value(self).expect("delegation claims serialization is infallible"),
        )
    }

    /// Sign and encode the exact metadata value accepted by runtime
    /// admission. The signer is owner-bound, so an issuer cannot accidentally
    /// be signed with another principal's key.
    pub async fn signed_metadata_value(
        &self,
        signer: &dyn CanonicalSigner,
    ) -> anyhow::Result<String> {
        self.validate_shape()?;
        if signer.owner_ura() != self.issuer_ura {
            anyhow::bail!(
                "delegation signer `{}` does not match issuer `{}`",
                signer.owner_ura(),
                self.issuer_ura
            );
        }
        let signature = signer
            .sign_canonical(&self.canonical_payload_bytes())
            .await?;
        let wire = json!({
            "payload": self,
            "signature": BASE64_STANDARD.encode(signature.to_bytes()),
        });
        Ok(BASE64_STANDARD.encode(serde_json::to_vec(&wire)?))
    }

    pub(crate) fn validate_shape(&self) -> anyhow::Result<()> {
        let scalar_fields = [
            ("issuer_ura", self.issuer_ura.as_str()),
            ("subject_ura", self.subject_ura.as_str()),
            ("caller_ura", self.caller_ura.as_str()),
            ("audience", self.audience.as_str()),
        ];
        for (name, value) in scalar_fields {
            if value.is_empty() || value != value.trim() {
                anyhow::bail!("delegation {name} must be non-empty and trimmed");
            }
            crate::core::ura::parse_ura(value).map_err(|error| {
                anyhow::anyhow!("delegation {name} must be a canonical URA: {error}")
            })?;
        }
        if self.scopes.is_empty()
            || self
                .scopes
                .iter()
                .any(|scope| scope.is_empty() || scope != scope.trim())
        {
            anyhow::bail!("delegation scopes must contain at least one non-empty trimmed selector");
        }
        if self.issued_at_ms < 0 || self.expires_at_ms <= self.issued_at_ms {
            anyhow::bail!("delegation expiry must be later than its non-negative issue time");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityPredicate {
    governs_advertise: bool,
    governs_invoke: bool,
}

impl AuthorityPredicate {
    pub fn advertise_and_invoke() -> Self {
        Self {
            governs_advertise: true,
            governs_invoke: true,
        }
    }

    pub fn governs_advertise(&self) -> bool {
        self.governs_advertise
    }

    pub fn governs_invoke(&self) -> bool {
        self.governs_invoke
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityScope {
    /// Human-readable owner plane: device, authority, system-agent:<id>,
    /// agent:<id>, plugin:<id>.
    ///
    /// This is intentionally not a protocol owner. The protocol owner is the
    /// callee/owner URA resolved during advertisement/invocation.
    owner_projection: String,
    /// URA or stable local authority root that backs the binding. Before join
    /// this may be a local marker; once advertised externally it must resolve
    /// to a routable URA.
    authority_root: String,
}

impl AuthorityScope {
    pub fn new(
        owner_projection: impl Into<String>,
        authority_root: impl Into<String>,
    ) -> Result<Self, AbilityControlPlaneError> {
        let owner_projection = owner_projection.into();
        let authority_root = authority_root.into();
        if owner_projection.trim().is_empty() {
            return Err(AbilityControlPlaneError::EmptyAuthorityOwnerProjection);
        }
        if authority_root.trim().is_empty() {
            return Err(AbilityControlPlaneError::EmptyAuthorityRoot);
        }
        // The owner projection must be one of the canonical owner-plane
        // markers, and it is canonicalized so a marker can never be stored
        // with incidental whitespace that would split otherwise-equal scopes
        // across two map keys.
        let owner_projection = OwnerProjection::parse(owner_projection.trim())?.canonical();
        // The authority root backs runtime ability keys and proof facts, so it
        // must not carry surrounding whitespace or interior control characters
        // that would make a "format-like but route-unreal" key.
        if !is_valid_authority_root(&authority_root) {
            return Err(AbilityControlPlaneError::InvalidAuthorityRoot { authority_root });
        }
        Ok(Self {
            owner_projection,
            authority_root,
        })
    }

    pub fn owner_projection(&self) -> &str {
        &self.owner_projection
    }

    pub fn authority_root(&self) -> &str {
        &self.authority_root
    }
}

/// An authority root must be a trimmed, single-token string with no interior
/// whitespace or control characters. It may be a routable URA or a stable
/// local marker, so the character set stays permissive; only shapes that
/// cannot serve as a stable key are rejected.
fn is_valid_authority_root(authority_root: &str) -> bool {
    authority_root == authority_root.trim()
        && !authority_root.is_empty()
        && !authority_root.chars().any(char::is_whitespace)
        && !authority_root.chars().any(char::is_control)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityBindingKind {
    #[serde(rename = "self")]
    SelfBinding,
    HostedAgentDelegation,
}

impl AuthorityBindingKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SelfBinding => "self",
            Self::HostedAgentDelegation => "hosted_agent_delegation",
        }
    }
}

/// Unsigned local request to mint hosted-agent delegation authority.
///
/// What this is: a daemon-loopback control request from the local CLI to the
/// daemon transport layer. It names the hosted Agent whose local host authority
/// should be projected onto the current Axon invocation envelope.
///
/// What this is not: authorization proof. Handlers must never consume this
/// value directly; the transport layer must convert it into signed
/// [`HostedAgentDelegationClaims`] metadata first.
///
/// Invariant 1: `agent_ura` is always a canonical, trimmed user-owned Agent
/// URA. Device-sponsored SystemAgent URAs and daemon-local `_system.local`
/// identities are not hosted-agent principals.
/// Invariant 2: serialization is JSON, not a bare string, so the metadata
/// grammar can evolve without overloading arbitrary URA text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostedAgentDelegationRequest {
    agent_ura: String,
}

impl HostedAgentDelegationRequest {
    pub fn new(agent_ura: impl Into<String>) -> anyhow::Result<Self> {
        let request = Self {
            agent_ura: agent_ura.into(),
        };
        request.validate()?;
        Ok(request)
    }

    pub fn from_metadata_value(raw: &str) -> anyhow::Result<Self> {
        let request: Self = serde_json::from_str(raw).map_err(|err| {
            anyhow::anyhow!("invalid hosted-agent delegation request JSON: {err}")
        })?;
        request.validate()?;
        Ok(request)
    }

    pub fn metadata_value(&self) -> anyhow::Result<String> {
        serde_json::to_string(self)
            .map_err(|err| anyhow::anyhow!("encode hosted-agent delegation request: {err}"))
    }

    pub fn agent_ura(&self) -> &str {
        self.agent_ura.as_str()
    }

    pub fn into_claims(
        self,
        signing_authority: impl Into<String>,
        envelope: HostedAgentDelegationEnvelopeBinding,
    ) -> anyhow::Result<HostedAgentDelegationClaims> {
        HostedAgentDelegationClaims::new(self.agent_ura, signing_authority, envelope)
    }

    fn validate(&self) -> anyhow::Result<()> {
        let agent_ura = self.agent_ura.trim();
        if agent_ura.is_empty() {
            anyhow::bail!("hosted-agent delegation request requires a non-empty Agent URA");
        }
        if agent_ura != self.agent_ura {
            anyhow::bail!("hosted-agent delegation request Agent URA must be trimmed");
        }
        validate_hosted_agent_ura(agent_ura, "hosted-agent delegation request")?;
        Ok(())
    }
}

/// Invocation-envelope facts that are signed into hosted-agent delegation.
///
/// This is intentionally a value object rather than five free strings at every
/// call site. The caller/callee/subject/nonce/route-ability tuple is a local
/// product-authority binding; passing it as one object keeps signing and
/// verification from drifting by argument order.
///
/// Invariant 1: `route_ability` is the public ability name from the route, not
/// an Axon descriptor ref. Descriptor versions are runtime proof facts and do
/// not belong in this local host-authority token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostedAgentDelegationEnvelopeBinding {
    wire_caller_ura: String,
    wire_callee_ura: String,
    host_device_ura: String,
    wire_subject_ura: String,
    wire_invocation_nonce_hex: String,
    route_ability: String,
}

impl HostedAgentDelegationEnvelopeBinding {
    pub fn new(
        wire_caller_ura: impl Into<String>,
        wire_callee_ura: impl Into<String>,
        host_device_ura: impl Into<String>,
        wire_subject_ura: impl Into<String>,
        wire_invocation_nonce_hex: impl Into<String>,
        route_ability: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let binding = Self {
            wire_caller_ura: wire_caller_ura.into(),
            wire_callee_ura: wire_callee_ura.into(),
            host_device_ura: host_device_ura.into(),
            wire_subject_ura: wire_subject_ura.into(),
            wire_invocation_nonce_hex: wire_invocation_nonce_hex.into(),
            route_ability: route_ability.into(),
        };
        binding.validate()?;
        Ok(binding)
    }

    fn validate(&self) -> anyhow::Result<()> {
        if self.wire_caller_ura.trim().is_empty()
            || self.wire_callee_ura.trim().is_empty()
            || self.host_device_ura.trim().is_empty()
            || self.wire_subject_ura.trim().is_empty()
            || self.wire_invocation_nonce_hex.trim().is_empty()
            || self.route_ability.trim().is_empty()
        {
            anyhow::bail!("hosted-agent delegation envelope binding fields must be non-empty");
        }
        validate_host_device_ura(
            self.host_device_ura.trim(),
            "hosted-agent delegation envelope binding",
        )?;
        validate_delegation_callee_ura(
            self.wire_callee_ura.trim(),
            "hosted-agent delegation envelope binding",
        )?;
        if self.route_ability.trim().contains('@') {
            anyhow::bail!(
                "hosted-agent delegation route_ability must be a public route name, not a descriptor ref"
            );
        }
        let nonce = hex::decode(self.wire_invocation_nonce_hex.trim()).map_err(|err| {
            anyhow::anyhow!("hosted-agent delegation invocation nonce must be hex: {err}")
        })?;
        if nonce.len() != 16 {
            anyhow::bail!(
                "hosted-agent delegation invocation nonce must decode to 16 bytes, got {}",
                nonce.len()
            );
        }
        Ok(())
    }

    pub fn caller_ura(&self) -> &str {
        self.wire_caller_ura.trim()
    }

    fn callee_ura(&self) -> &str {
        self.wire_callee_ura.trim()
    }

    fn host_device_ura(&self) -> &str {
        self.host_device_ura.trim()
    }

    fn subject_ura(&self) -> &str {
        self.wire_subject_ura.trim()
    }

    fn invocation_nonce_hex(&self) -> &str {
        self.wire_invocation_nonce_hex.trim()
    }

    fn route_ability(&self) -> &str {
        self.route_ability.trim()
    }
}

/// Signed local control claim that allows the host process to act for one
/// hosted Agent on administrative ability surfaces.
///
/// Invariant 1: every field that dispatch later compares with the invocation
/// envelope is inside the signed canonical payload.
///
/// Invariant 2: this is local control metadata only. Public transport
/// admission rejects the metadata key before this verifier can run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostedAgentDelegationClaims {
    kind: String,
    agent_ura: String,
    signing_authority: String,
    wire_caller_ura: String,
    wire_callee_ura: String,
    host_device_ura: String,
    wire_subject_ura: String,
    wire_invocation_nonce_hex: String,
    route_ability: String,
}

impl HostedAgentDelegationClaims {
    pub fn new(
        agent_ura: impl Into<String>,
        signing_authority: impl Into<String>,
        envelope: HostedAgentDelegationEnvelopeBinding,
    ) -> anyhow::Result<Self> {
        let claims = Self {
            kind: "hosted_agent".to_string(),
            agent_ura: agent_ura.into(),
            signing_authority: signing_authority.into(),
            wire_caller_ura: envelope.wire_caller_ura,
            wire_callee_ura: envelope.wire_callee_ura,
            host_device_ura: envelope.host_device_ura,
            wire_subject_ura: envelope.wire_subject_ura,
            wire_invocation_nonce_hex: envelope.wire_invocation_nonce_hex,
            route_ability: envelope.route_ability,
        };
        claims.validate()?;
        Ok(claims)
    }

    pub fn signing_payload_bytes(&self, signer_ura: &str) -> Vec<u8> {
        canonical_json_bytes(&json!({
            "claims": self,
            "signer_ura": signer_ura,
        }))
    }

    pub fn signed_metadata_value(
        &self,
        signer_ura: &str,
        signature: &Signature,
    ) -> anyhow::Result<String> {
        let token = SignedHostedAgentDelegation {
            claims: self.clone(),
            signer_ura: signer_ura.to_string(),
            signature_b64: BASE64_STANDARD.encode(signature.to_bytes()),
        };
        serde_json::to_string(&token)
            .map_err(|err| anyhow::anyhow!("encode hosted-agent delegation token: {err}"))
    }

    fn validate(&self) -> anyhow::Result<()> {
        if self.kind.trim().is_empty()
            || self.agent_ura.trim().is_empty()
            || self.signing_authority.trim().is_empty()
            || self.wire_caller_ura.trim().is_empty()
            || self.wire_callee_ura.trim().is_empty()
            || self.host_device_ura.trim().is_empty()
            || self.wire_subject_ura.trim().is_empty()
            || self.wire_invocation_nonce_hex.trim().is_empty()
            || self.route_ability.trim().is_empty()
        {
            anyhow::bail!("hosted-agent delegation claims fields must be non-empty");
        }
        validate_host_device_ura(
            self.host_device_ura.trim(),
            "hosted-agent delegation claims",
        )?;
        if self.route_ability.trim().contains('@') {
            anyhow::bail!(
                "hosted-agent delegation route_ability must be a public route name, not a descriptor ref"
            );
        }
        validate_hosted_agent_ura(self.agent_ura.trim(), "hosted-agent delegation claims")?;
        let nonce = hex::decode(self.wire_invocation_nonce_hex.trim()).map_err(|err| {
            anyhow::anyhow!("hosted-agent delegation invocation nonce must be hex: {err}")
        })?;
        if nonce.len() != 16 {
            anyhow::bail!(
                "hosted-agent delegation invocation nonce must decode to 16 bytes, got {}",
                nonce.len()
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SignedHostedAgentDelegation {
    claims: HostedAgentDelegationClaims,
    signer_ura: String,
    signature_b64: String,
}

/// Runtime-local authority fact for a local hosted-agent call.
///
/// Axon proves the invocation envelope. EasyNet proves that an operator
/// invoking through the host device is allowed to act for one hosted Agent
/// on local administrative surfaces such as `meta.teach` and `meta.acquire`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostedAgentDelegationContext {
    agent_ura: String,
    signing_authority: String,
    wire_caller_ura: String,
    wire_callee_ura: String,
    host_device_ura: String,
    wire_subject_ura: String,
    wire_invocation_nonce_hex: String,
    route_ability: String,
}

impl HostedAgentDelegationContext {
    const HOST_DEVICE_SIGNING_AUTHORITY: &'static str = "host_device";

    pub fn from_signed_metadata(
        raw: &str,
        envelope: &HostedAgentDelegationEnvelopeBinding,
        verifying_key: VerifyingKey,
    ) -> anyhow::Result<Self> {
        let token: SignedHostedAgentDelegation = serde_json::from_str(raw)
            .map_err(|err| anyhow::anyhow!("invalid hosted-agent delegation token JSON: {err}"))?;
        token.claims.validate()?;
        let signer_ura = token.signer_ura.trim();
        if signer_ura.is_empty() {
            anyhow::bail!("hosted-agent delegation token signer_ura must be non-empty");
        }
        if signer_ura != envelope.caller_ura() {
            anyhow::bail!(
                "hosted-agent delegation signer {signer_ura:?} does not match envelope caller {:?}",
                envelope.caller_ura()
            );
        }
        let signature_bytes = BASE64_STANDARD
            .decode(token.signature_b64.as_bytes())
            .map_err(|err| anyhow::anyhow!("decode hosted-agent delegation signature: {err}"))?;
        let signature_array: [u8; 64] = signature_bytes.as_slice().try_into().map_err(|_| {
            anyhow::anyhow!(
                "hosted-agent delegation signature must be 64 bytes, got {}",
                signature_bytes.len()
            )
        })?;
        let signature = Signature::from_bytes(&signature_array);
        verifying_key
            .verify(&token.claims.signing_payload_bytes(signer_ura), &signature)
            .map_err(|err| anyhow::anyhow!("hosted-agent delegation signature invalid: {err}"))?;
        Self::from_bound_claims(token.claims, envelope)
    }

    fn from_bound_claims(
        claims: HostedAgentDelegationClaims,
        envelope: &HostedAgentDelegationEnvelopeBinding,
    ) -> anyhow::Result<Self> {
        if claims.kind != "hosted_agent" {
            anyhow::bail!(
                "hosted-agent delegation token uses unsupported kind {:?}",
                claims.kind
            );
        }
        let agent_ura = claims.agent_ura.trim();
        let signing_authority = claims.signing_authority.trim();
        let wire_caller_ura = claims.wire_caller_ura.trim();
        let wire_callee_ura = claims.wire_callee_ura.trim();
        let host_device_ura = claims.host_device_ura.trim();
        let wire_subject_ura = claims.wire_subject_ura.trim();
        let wire_invocation_nonce_hex = claims.wire_invocation_nonce_hex.trim();
        let route_ability = claims.route_ability.trim();
        if agent_ura.is_empty()
            || signing_authority.is_empty()
            || wire_caller_ura.is_empty()
            || wire_callee_ura.is_empty()
            || host_device_ura.is_empty()
            || wire_subject_ura.is_empty()
            || wire_invocation_nonce_hex.is_empty()
            || route_ability.is_empty()
        {
            anyhow::bail!("hosted-agent delegation metadata fields must be non-empty");
        }
        validate_hosted_agent_ura(agent_ura, "hosted-agent delegation metadata")?;
        validate_host_device_ura(host_device_ura, "hosted-agent delegation metadata")?;
        if wire_caller_ura != envelope.caller_ura()
            || wire_callee_ura != envelope.callee_ura()
            || host_device_ura != envelope.host_device_ura()
            || wire_subject_ura != envelope.subject_ura()
            || wire_invocation_nonce_hex != envelope.invocation_nonce_hex()
            || route_ability != envelope.route_ability()
        {
            anyhow::bail!(
                "hosted-agent delegation metadata does not match the signed invocation envelope"
            );
        }
        Ok(Self {
            agent_ura: agent_ura.to_string(),
            signing_authority: signing_authority.to_string(),
            wire_caller_ura: wire_caller_ura.to_string(),
            wire_callee_ura: wire_callee_ura.to_string(),
            host_device_ura: host_device_ura.to_string(),
            wire_subject_ura: wire_subject_ura.to_string(),
            wire_invocation_nonce_hex: wire_invocation_nonce_hex.to_string(),
            route_ability: route_ability.to_string(),
        })
    }

    pub fn authorize(
        &self,
        expected_agent_ura: &str,
        persisted_signing_authority: &str,
        expected_ability: &str,
    ) -> anyhow::Result<HostedAgentAuthority> {
        if self.agent_ura != expected_agent_ura {
            anyhow::bail!(
                "{expected_ability} hosted-agent authority targets {}, expected {}",
                self.agent_ura,
                expected_agent_ura
            );
        }
        if self.route_ability != expected_ability {
            anyhow::bail!(
                "{expected_ability} hosted-agent authority was issued for {}, expected {expected_ability}",
                self.route_ability
            );
        }
        if self.wire_caller_ura.trim().is_empty() || self.wire_subject_ura.trim().is_empty() {
            anyhow::bail!(
                "{expected_ability} hosted-agent authority is missing bound caller or subject"
            );
        }
        if self.signing_authority != Self::HOST_DEVICE_SIGNING_AUTHORITY {
            anyhow::bail!(
                "{expected_ability} hosted-agent authority uses unsupported signing_authority {:?}",
                self.signing_authority
            );
        }
        validate_host_device_ura(&self.host_device_ura, expected_ability)?;
        let expected_host_authority = format!("hosted_by:{}", self.host_device_ura);
        if persisted_signing_authority != expected_host_authority {
            anyhow::bail!(
                "{expected_ability} hosted-agent authority host {} does not match persisted authority {}",
                self.host_device_ura,
                persisted_signing_authority
            );
        }
        Ok(HostedAgentAuthority {
            agent_ura: self.agent_ura.clone(),
            host_device_ura: self.host_device_ura.clone(),
            ability: self.route_ability.clone(),
            binding_kind: AuthorityBindingKind::HostedAgentDelegation,
        })
    }
}

fn validate_hosted_agent_ura(agent_ura: &str, context: &str) -> anyhow::Result<()> {
    let parsed = crate::core::ura::parse_ura(agent_ura)
        .map_err(|err| anyhow::anyhow!("{context} Agent URA is invalid: {err}"))?;
    if parsed.kind != crate::core::ura::URAKind::Agent {
        anyhow::bail!("{context} requires an Agent URA, got {:?}", parsed.kind);
    }
    if agent_ura == crate::core::ura::LOCAL_SYSTEM_AGENT_URA || parsed.realm == "_system" {
        anyhow::bail!("{context} cannot target daemon-local system identity {agent_ura:?}");
    }
    if parsed.agent_ids().is_none() {
        anyhow::bail!(
            "{context} requires a user-owned Agent URA; device-sponsored SystemAgent URAs are not hosted agents"
        );
    }
    Ok(())
}

fn validate_host_device_ura(host_device_ura: &str, context: &str) -> anyhow::Result<()> {
    let parsed = crate::core::ura::parse_ura(host_device_ura).map_err(|err| {
        anyhow::anyhow!("{context} hosted-agent host Device URA is invalid: {err}")
    })?;
    if parsed.kind != crate::core::ura::URAKind::Device {
        anyhow::bail!(
            "{context} hosted-agent host must be a Device URA, got {:?}",
            parsed.kind
        );
    }
    Ok(())
}

fn validate_delegation_callee_ura(wire_callee_ura: &str, context: &str) -> anyhow::Result<()> {
    let parsed = crate::core::ura::parse_ura(wire_callee_ura)
        .map_err(|err| anyhow::anyhow!("{context} wire callee URA is invalid: {err}"))?;
    if parsed.kind == crate::core::ura::URAKind::Device {
        anyhow::bail!(
            "{context} wire callee must be an Agent/SystemAgent or Authority; \
             host Device belongs in host_device_ura, got Device callee {wire_callee_ura:?}"
        );
    }
    match parsed.kind {
        crate::core::ura::URAKind::Agent | crate::core::ura::URAKind::Authority => Ok(()),
        other => anyhow::bail!(
            "{context} wire callee must be an Agent/SystemAgent or Authority, got {other:?}"
        ),
    }
}

/// Project an Axon runtime descriptor ref onto the public ability name used by
/// hosted-agent delegation metadata.
///
/// Runtime dispatch keeps the descriptor ref in the Axon envelope. Local
/// EasyNet authorization must compare the stable public route ability instead
/// of embedding descriptor-version facts in the delegation token.
pub(crate) fn public_route_ability_from_descriptor_ref(
    descriptor_ref: &str,
) -> anyhow::Result<String> {
    let ability_ura =
        crate::daemon::axon_bridge::descriptor_ref::ability_ura_from_descriptor_ref(descriptor_ref)
            .map_err(|err| anyhow::anyhow!("invalid runtime ability descriptor ref: {err}"))?;
    let selector = crate::core::ura::AbilitySelector::parse(&ability_ura)?;
    Ok(selector.public_name().to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostedAgentAuthority {
    agent_ura: String,
    host_device_ura: String,
    ability: String,
    binding_kind: AuthorityBindingKind,
}

impl HostedAgentAuthority {
    pub fn agent_ura(&self) -> &str {
        &self.agent_ura
    }

    pub fn host_device_ura(&self) -> &str {
        &self.host_device_ura
    }

    pub fn ability(&self) -> &str {
        &self.ability
    }

    pub fn binding_kind(&self) -> AuthorityBindingKind {
        self.binding_kind
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityBinding {
    ability: String,
    descriptor_version: String,
    call_mode: CallMode,
    scope: AuthorityScope,
    predicate: AuthorityPredicate,
    binding_kind: AuthorityBindingKind,
    invoke_policy_ref: String,
    invoke_policy_hash: [u8; 32],
}

impl AuthorityBinding {
    pub fn local_self_for_descriptor(
        ability: impl Into<String>,
        scope: AuthorityScope,
        descriptor: &crate::daemon::ability::descriptors::AbilityDescriptor,
    ) -> Result<Self, AbilityControlPlaneError> {
        let ability = ability.into();
        let descriptor_version = descriptor.version.clone();
        let call_mode = descriptor.call_mode();
        if ability.trim().is_empty() {
            return Err(AbilityControlPlaneError::EmptyAuthorityAbility);
        }
        if !is_valid_ability_name(&ability) {
            return Err(AbilityControlPlaneError::InvalidAuthorityAbility { ability });
        }
        if descriptor_version.trim().is_empty() {
            return Err(AbilityControlPlaneError::EmptyAuthorityDescriptorVersion);
        }
        if !is_valid_descriptor_version(&descriptor_version) {
            return Err(
                AbilityControlPlaneError::InvalidAuthorityDescriptorVersion {
                    version: descriptor_version,
                },
            );
        }
        Ok(Self {
            ability,
            descriptor_version,
            call_mode,
            scope,
            predicate: AuthorityPredicate::advertise_and_invoke(),
            binding_kind: AuthorityBindingKind::SelfBinding,
            invoke_policy_ref: DEFAULT_INVOKE_POLICY_REF.to_string(),
            invoke_policy_hash: descriptor.access_policy_hash_bytes(),
        })
    }

    pub fn ability(&self) -> &str {
        &self.ability
    }

    pub fn descriptor_version(&self) -> &str {
        &self.descriptor_version
    }

    pub fn call_mode(&self) -> CallMode {
        self.call_mode
    }

    pub fn scope(&self) -> &AuthorityScope {
        &self.scope
    }

    pub fn predicate(&self) -> &AuthorityPredicate {
        &self.predicate
    }

    pub fn binding_kind(&self) -> AuthorityBindingKind {
        self.binding_kind
    }

    pub fn invoke_policy_ref(&self) -> &str {
        &self.invoke_policy_ref
    }

    pub fn invoke_policy_hash(&self) -> [u8; 32] {
        self.invoke_policy_hash
    }

    pub fn invoke_policy_hash_prefixed_hex(&self) -> String {
        format!("sha256:{}", hex::encode(self.invoke_policy_hash))
    }

    pub fn key(&self) -> AbilityControlPlaneKey {
        AbilityControlPlaneKey::for_authority(self)
    }
}

#[derive(Debug, Default, Clone)]
pub struct AuthorityBindingRegistry {
    bindings: BTreeMap<AbilityControlPlaneKey, AuthorityBinding>,
}

impl AuthorityBindingRegistry {
    pub(crate) fn bind(&mut self, binding: AuthorityBinding) {
        self.bindings.insert(binding.key(), binding);
    }

    pub(crate) fn get(&self, key: &AbilityControlPlaneKey) -> Option<&AuthorityBinding> {
        self.bindings.get(key)
    }

    pub(crate) fn remove_matching(
        &mut self,
        mut predicate: impl FnMut(&AbilityControlPlaneKey) -> bool,
    ) -> bool {
        let before = self.bindings.len();
        self.bindings.retain(|key, _| !predicate(key));
        self.bindings.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCAL_DEVICE_URA: &str = "easynet:///r/default/device/local";
    const LOCAL_SYSTEM_AGENT_URA: &str =
        "easynet:///r/default/agent/device.local.runtime-introspection";

    fn descriptor(name: &str) -> crate::daemon::ability::descriptors::AbilityDescriptor {
        crate::daemon::ability::descriptors::AbilityDescriptor::new(
            name,
            LOCAL_SYSTEM_AGENT_URA,
            crate::daemon::ability::descriptors::Visibility::Scoped,
            crate::daemon::ability::descriptors::AdmissionAction::Invoke,
        )
        .unwrap()
    }

    #[test]
    fn authority_predicate_covers_advertise_and_invoke() {
        let descriptor = descriptor("fs.read");
        let record = AuthorityBinding::local_self_for_descriptor(
            "fs.read",
            AuthorityScope::new("device", LOCAL_DEVICE_URA).unwrap(),
            &descriptor,
        )
        .unwrap();
        assert!(record.predicate().governs_advertise());
        assert!(record.predicate().governs_invoke());
    }

    #[test]
    fn authority_scope_rejects_empty_values_without_panicking() {
        assert_eq!(
            AuthorityScope::new("", LOCAL_DEVICE_URA).unwrap_err(),
            AbilityControlPlaneError::EmptyAuthorityOwnerProjection
        );
        assert_eq!(
            AuthorityScope::new("device", " ").unwrap_err(),
            AbilityControlPlaneError::EmptyAuthorityRoot
        );
        assert_eq!(
            AuthorityBinding::local_self_for_descriptor(
                "bad/name",
                AuthorityScope::new("device", LOCAL_DEVICE_URA).unwrap(),
                &descriptor("fs.read"),
            )
            .unwrap_err(),
            AbilityControlPlaneError::InvalidAuthorityAbility {
                ability: "bad/name".to_string()
            }
        );
        let mut invalid_version = descriptor("fs.read");
        invalid_version.version = "v1".to_string();
        assert_eq!(
            AuthorityBinding::local_self_for_descriptor(
                "fs.read",
                AuthorityScope::new("device", LOCAL_DEVICE_URA).unwrap(),
                &invalid_version,
            )
            .unwrap_err(),
            AbilityControlPlaneError::InvalidAuthorityDescriptorVersion {
                version: "v1".to_string()
            }
        );
    }

    #[test]
    fn authority_scope_accepts_every_canonical_owner_marker() {
        for marker in [
            "device",
            "authority",
            "system-agent:agent-management",
            "agent:codex",
            "plugin:fs.read",
        ] {
            let scope = AuthorityScope::new(marker, LOCAL_DEVICE_URA)
                .unwrap_or_else(|err| panic!("{marker} must be a valid owner projection: {err}"));
            assert_eq!(scope.owner_projection(), marker);
        }
    }

    #[test]
    fn authority_scope_authority_marker_projects_realm_authority_state() {
        let projection = OwnerProjection::parse("authority").expect("authority marker is accepted");

        assert_eq!(projection, OwnerProjection::RealmAuthority);
        assert_eq!(projection.canonical(), "authority");
    }

    #[test]
    fn authority_scope_rejects_retired_hub_owner_marker() {
        assert_eq!(
            AuthorityScope::new("hub", LOCAL_DEVICE_URA).unwrap_err(),
            AbilityControlPlaneError::InvalidAuthorityOwnerProjection {
                projection: "hub".to_string()
            }
        );
    }

    #[test]
    fn authority_scope_canonicalizes_surrounding_whitespace_in_owner_marker() {
        let scope = AuthorityScope::new("  agent:codex  ", LOCAL_DEVICE_URA).unwrap();
        assert_eq!(
            scope.owner_projection(),
            "agent:codex",
            "owner projection must be trimmed so equal scopes share one map key"
        );
    }

    #[test]
    fn authority_scope_rejects_non_canonical_owner_markers() {
        for marker in [
            "realm",
            "agent",
            "agent:",
            "device:1",
            "user:u-1",
            "user: ",
            "agent:bad id",
        ] {
            assert_eq!(
                AuthorityScope::new(marker, LOCAL_DEVICE_URA).unwrap_err(),
                AbilityControlPlaneError::InvalidAuthorityOwnerProjection {
                    projection: marker.trim().to_string()
                },
                "{marker} must be rejected as a non-canonical owner projection"
            );
        }
    }

    #[test]
    fn authority_scope_rejects_authority_root_with_interior_whitespace() {
        assert_eq!(
            AuthorityScope::new("device", "easynet:///r/default/device local").unwrap_err(),
            AbilityControlPlaneError::InvalidAuthorityRoot {
                authority_root: "easynet:///r/default/device local".to_string()
            },
        );
    }

    #[test]
    fn invoke_policy_hash_changes_with_manifest_access() {
        let input = serde_json::json!({"type": "object"});
        let base =
            crate::daemon::ability::manifest::AbilityManifest::new("quote", "quote", input.clone())
                .unwrap();
        let restricted =
            crate::daemon::ability::manifest::AbilityManifest::new("quote", "quote", input)
                .unwrap()
                .with_access(crate::daemon::ability::manifest::AccessPolicy {
                    visibility: crate::daemon::ability::manifest::ManifestAccessScope::Selfish,
                    allow_callers: None,
                    deny_callers: None,
                })
                .unwrap();

        let base_descriptor =
            crate::daemon::ability::descriptors::AbilityDescriptor::from_registry_manifest(
                "mentor.quote",
                "easynet:///r/default/agent/u.mentor",
                CallMode::Rpc,
                crate::daemon::ability::descriptors::AdmissionAction::Invoke,
                &base,
            )
            .unwrap();
        let restricted_descriptor =
            crate::daemon::ability::descriptors::AbilityDescriptor::from_registry_manifest(
                "mentor.quote",
                "easynet:///r/default/agent/u.mentor",
                CallMode::Rpc,
                crate::daemon::ability::descriptors::AdmissionAction::Invoke,
                &restricted,
            )
            .unwrap();
        let scope =
            AuthorityScope::new("agent:mentor", "easynet:///r/default/agent/u.mentor").unwrap();
        let base_record = AuthorityBinding::local_self_for_descriptor(
            "mentor.quote",
            scope.clone(),
            &base_descriptor,
        )
        .unwrap();
        let restricted_record = AuthorityBinding::local_self_for_descriptor(
            "mentor.quote",
            scope,
            &restricted_descriptor,
        )
        .unwrap();
        assert_ne!(
            base_record.invoke_policy_hash(),
            restricted_record.invoke_policy_hash(),
            "authority binding must prove which invoke access policy it governs"
        );
    }

    #[test]
    fn hosted_agent_delegation_token_verifies_bound_claims() {
        use ed25519_dalek::Signer as _;

        let signer = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let caller = crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA;
        let callee = crate::core::ura::device_agent_ura("default", "local", "ability-management");
        let host_device_ura = crate::core::ura::device_ura("default", "local");
        let subject = "easynet:///r/default/device/local";
        let nonce_hex = hex::encode([0x42u8; 16]);
        let ability = "meta.acquire";
        let agent_ura = crate::core::ura::agent_ura("default", "u", "apprentice");
        let envelope = HostedAgentDelegationEnvelopeBinding::new(
            caller,
            callee.as_str(),
            host_device_ura.as_str(),
            subject,
            nonce_hex.as_str(),
            ability,
        )
        .unwrap();
        let claims =
            HostedAgentDelegationClaims::new(agent_ura.as_str(), "host_device", envelope.clone())
                .unwrap();
        let signature = signer.sign(&claims.signing_payload_bytes(caller));
        let raw = claims.signed_metadata_value(caller, &signature).unwrap();

        let context = HostedAgentDelegationContext::from_signed_metadata(
            &raw,
            &envelope,
            signer.verifying_key(),
        )
        .unwrap();

        let authority = context
            .authorize(
                agent_ura.as_str(),
                "hosted_by:easynet:///r/default/device/local",
                "meta.acquire",
            )
            .unwrap();
        assert_eq!(authority.ability(), "meta.acquire");
    }

    #[test]
    fn hosted_agent_delegation_binds_system_agent_callee_to_separate_host_device() {
        use ed25519_dalek::Signer as _;

        let signer = ed25519_dalek::SigningKey::from_bytes(&[0x17; 32]);
        let caller = crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA;
        let wire_callee =
            crate::core::ura::device_agent_ura("default", "local", "ability-management");
        let host_device_ura = crate::core::ura::device_ura("default", "local");
        let subject = host_device_ura.as_str();
        let nonce_hex = hex::encode([0x27u8; 16]);
        let ability = "meta.acquire";
        let agent_ura = crate::core::ura::agent_ura("default", "u", "apprentice");
        let envelope = HostedAgentDelegationEnvelopeBinding::new(
            caller,
            wire_callee.as_str(),
            host_device_ura.as_str(),
            subject,
            nonce_hex.as_str(),
            ability,
        )
        .unwrap();
        let claims =
            HostedAgentDelegationClaims::new(agent_ura.as_str(), "host_device", envelope.clone())
                .unwrap();
        let signature = signer.sign(&claims.signing_payload_bytes(caller));
        let raw = claims.signed_metadata_value(caller, &signature).unwrap();
        let context = HostedAgentDelegationContext::from_signed_metadata(
            &raw,
            &envelope,
            signer.verifying_key(),
        )
        .unwrap();

        let authority = context
            .authorize(
                agent_ura.as_str(),
                "hosted_by:easynet:///r/default/device/local",
                ability,
            )
            .unwrap();

        assert_eq!(authority.host_device_ura(), host_device_ura);
        assert_ne!(authority.host_device_ura(), wire_callee);
        let err = context
            .authorize(
                agent_ura.as_str(),
                &format!("hosted_by:{wire_callee}"),
                ability,
            )
            .unwrap_err();
        assert!(err.to_string().contains("does not match"), "{err}");
    }

    #[test]
    fn hosted_agent_delegation_request_round_trips_as_json() {
        let agent_ura = crate::core::ura::agent_ura("default", "u", "apprentice");
        let request = HostedAgentDelegationRequest::new(agent_ura.as_str()).unwrap();
        let raw = request.metadata_value().unwrap();

        let decoded = HostedAgentDelegationRequest::from_metadata_value(&raw).unwrap();

        assert_eq!(decoded.agent_ura(), agent_ura);
        assert!(
            raw.contains("agent_ura"),
            "request metadata must remain structured JSON"
        );
    }

    #[test]
    fn hosted_agent_delegation_request_rejects_non_agent_ura() {
        let err =
            HostedAgentDelegationRequest::new("easynet:///r/default/device/local").unwrap_err();

        assert!(err.to_string().contains("Agent URA"), "{err}");
    }

    #[test]
    fn hosted_agent_delegation_request_rejects_device_sponsored_system_agent() {
        let err = HostedAgentDelegationRequest::new(crate::core::ura::device_agent_ura(
            "default",
            "device-a",
            "system-worker",
        ))
        .unwrap_err();

        assert!(err.to_string().contains("SystemAgent"), "{err}");
    }

    #[test]
    fn hosted_agent_delegation_request_rejects_local_system_agent() {
        let err = HostedAgentDelegationRequest::new(crate::core::ura::LOCAL_SYSTEM_AGENT_URA)
            .unwrap_err();

        assert!(err.to_string().contains("system identity"), "{err}");
    }

    #[test]
    fn hosted_agent_delegation_claims_reject_system_agent_direct_construction() {
        let envelope = HostedAgentDelegationEnvelopeBinding::new(
            crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
            crate::core::ura::device_agent_ura("default", "local", "ability-management"),
            "easynet:///r/default/device/local",
            "easynet:///r/default/device/local",
            hex::encode([0x45u8; 16]),
            "meta.acquire",
        )
        .unwrap();

        let err = HostedAgentDelegationClaims::new(
            crate::core::ura::device_agent_ura("default", "device-a", "system-worker"),
            "host_device",
            envelope,
        )
        .unwrap_err();

        assert!(err.to_string().contains("SystemAgent"), "{err}");
    }

    #[test]
    fn hosted_agent_delegation_request_rejects_unknown_fields() {
        let agent_ura = crate::core::ura::agent_ura("default", "u", "apprentice");
        let raw = serde_json::json!({
            "agent_ura": agent_ura,
            "legacy_scope": "hosted_by:easynet:///r/default/device/local"
        })
        .to_string();

        let err = HostedAgentDelegationRequest::from_metadata_value(&raw)
            .expect_err("hosted-agent delegation requests must reject shadow authority fields");

        assert!(err.to_string().contains("legacy_scope"), "{err}");
    }

    #[test]
    fn hosted_agent_delegation_token_rejects_envelope_drift() {
        use ed25519_dalek::Signer as _;

        let signer = ed25519_dalek::SigningKey::from_bytes(&[8u8; 32]);
        let caller = crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA;
        let agent_ura = crate::core::ura::agent_ura("default", "u", "apprentice");
        let nonce_hex = hex::encode([0x24u8; 16]);
        let ability = "meta.acquire";
        let wire_callee =
            crate::core::ura::device_agent_ura("default", "local", "ability-management");
        let drifted_wire_callee =
            crate::core::ura::device_agent_ura("default", "local", "runtime-control");
        let host_device_ura = crate::core::ura::device_ura("default", "local");
        let envelope = HostedAgentDelegationEnvelopeBinding::new(
            caller,
            wire_callee.as_str(),
            host_device_ura.as_str(),
            "easynet:///r/default/device/local",
            nonce_hex.as_str(),
            ability,
        )
        .unwrap();
        let claims = HostedAgentDelegationClaims::new(agent_ura, "host_device", envelope).unwrap();
        let signature = signer.sign(&claims.signing_payload_bytes(caller));
        let raw = claims.signed_metadata_value(caller, &signature).unwrap();
        let drifted_envelope = HostedAgentDelegationEnvelopeBinding::new(
            caller,
            drifted_wire_callee.as_str(),
            host_device_ura.as_str(),
            "easynet:///r/default/device/local",
            nonce_hex.as_str(),
            ability,
        )
        .unwrap();

        let err = HostedAgentDelegationContext::from_signed_metadata(
            &raw,
            &drifted_envelope,
            signer.verifying_key(),
        )
        .unwrap_err();

        assert!(
            err.to_string().contains("signed invocation envelope"),
            "{err}"
        );
    }

    #[test]
    fn hosted_agent_delegation_token_rejects_nonce_replay() {
        use ed25519_dalek::Signer as _;

        let signer = ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]);
        let caller = crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA;
        let callee = crate::core::ura::device_agent_ura("default", "local", "ability-management");
        let host_device_ura = crate::core::ura::device_ura("default", "local");
        let subject = "easynet:///r/default/device/local";
        let nonce_hex = hex::encode([0x33u8; 16]);
        let ability = "meta.acquire";
        let envelope = HostedAgentDelegationEnvelopeBinding::new(
            caller,
            callee.as_str(),
            host_device_ura.as_str(),
            subject,
            nonce_hex.as_str(),
            ability,
        )
        .unwrap();
        let claims = HostedAgentDelegationClaims::new(
            crate::core::ura::agent_ura("default", "u", "apprentice"),
            "host_device",
            envelope,
        )
        .unwrap();
        let signature = signer.sign(&claims.signing_payload_bytes(caller));
        let raw = claims.signed_metadata_value(caller, &signature).unwrap();
        let replayed_nonce_hex = hex::encode([0x34u8; 16]);
        let replayed_envelope = HostedAgentDelegationEnvelopeBinding::new(
            caller,
            callee.as_str(),
            host_device_ura.as_str(),
            subject,
            replayed_nonce_hex.as_str(),
            ability,
        )
        .unwrap();

        let err = HostedAgentDelegationContext::from_signed_metadata(
            &raw,
            &replayed_envelope,
            signer.verifying_key(),
        )
        .unwrap_err();

        assert!(
            err.to_string().contains("signed invocation envelope"),
            "{err}"
        );
    }

    #[test]
    fn hosted_agent_delegation_rejects_descriptor_ref_in_route_ability() {
        let wire_callee =
            crate::core::ura::device_agent_ura("default", "local", "ability-management");
        let descriptor_ref = format!(
            "{}@1.0.0",
            crate::core::ura::owner_ability_ura(&wire_callee, "meta.acquire").unwrap()
        );

        let err = HostedAgentDelegationEnvelopeBinding::new(
            crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
            wire_callee,
            "easynet:///r/default/device/local",
            "easynet:///r/default/device/local",
            hex::encode([0x33u8; 16]),
            descriptor_ref,
        )
        .unwrap_err();

        assert!(err.to_string().contains("public route name"), "{err}");
    }

    #[test]
    fn hosted_agent_delegation_rejects_device_callee() {
        let err = HostedAgentDelegationEnvelopeBinding::new(
            crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
            "easynet:///r/default/device/local",
            "easynet:///r/default/device/local",
            "easynet:///r/default/device/local",
            hex::encode([0x33u8; 16]),
            "meta.acquire",
        )
        .unwrap_err();

        assert!(err.to_string().contains("host_device_ura"), "{err}");
    }
}
