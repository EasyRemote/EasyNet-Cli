// EasyNet CLI — Axon invocation wire builders
// ===========================================
//
// File: src/daemon/invocation/invocation_wire.rs
// Description: Product-policy projection into Axon-owned canonical
//              Invocation envelopes and protocol transport requests.
//
// Protocol Responsibility
// -----------------------
// Axon owns the canonical seven-tuple and its descriptor-bound form.
// This module validates product URAs, requires an explicit freshness/
// causal derivation policy, and projects Axon's completed canonical
// envelope onto the protobuf carrier.
//
// Implementation Approach
// -----------------------
// Keep the API deliberately small. Every constructor requires a named
// `InvocationDerivationPolicy`; no constructor silently invents nonce
// or causal placement. Canonical assembly happens only after ability
// and args are known.
//
// Usage Contract
// --------------
// Production call sites select a derivation policy and must not
// hand-build `Envelope` / `InvokeRequest` struct literals. Tests may
// still construct raw proto fixtures to exercise malformed shapes.
//
// Architectural Position
// ----------------------
// This is the wire-facade counterpart to `crate::core::ura` (canonical URA
// construction/parsing). Canonical invocation and receipt state remains owned
// by Axon SDK. It does not perform admission. When a caller provides a
// `SelfIdentity`, it can attach the caller signature over Axon's
// descriptor-bound canonical bytes so production daemon IPC has one
// wire-shape construction point.

use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use rand::RngCore;

use anyhow::Context as _;
use tonic::Status;

use crate::daemon::axon_bridge::proof_owner::descriptor_bound_canonical_bytes;
pub use axon_sdk::invocation::InvocationDerivationPolicy;
use axon_sdk::invocation::{
    AgentIdentity, CallerSignature, CanonicalEnvelopeBuilder, DescriptorBoundEnvelope,
    SubjectIdentity, UraProfile, WireEnvelopeMetadata,
};
use axon_sdk::pb::axon::v1::{
    invocation_target, AbilityTarget, ContentEnvelope, EntityRef, EntityRefKind, Envelope,
    EnvelopeOpen, InvocationTarget, InvokeRequest, InvokeServerStreamRequest, StreamDescriptor,
};

pub const DEFAULT_URA_PROFILE: &str = "axon-strict-v2";

pub(crate) const AUTHORITY_PROOF_METADATA_KEY: &str = "x-easynet-authority-proof";

/// Canonical issuer for root invocation derivation policies.
///
/// Callers that legitimately start a new root invocation use this named issuer
/// instead of mentioning `FreshRoot` inline. The resulting policy is still an
/// Axon-owned derivation primitive; this type only centralizes daemon/product
/// policy selection.
pub struct RootInvocationDerivationIssuer;

impl RootInvocationDerivationIssuer {
    #[must_use]
    pub fn fresh_root() -> InvocationDerivationPolicy {
        InvocationDerivationPolicy::FreshRoot
    }
}

#[derive(Debug, Clone)]
pub struct ProtoEnvelope {
    canonical: CanonicalEnvelopeBuilder,
    wire_metadata: WireEnvelopeMetadata,
}

/// Daemon-owned local loopback request projection.
///
/// This value is intentionally not the public `DaemonInvocation` SDK builder:
/// local CLI loopback submits route names and lets the daemon dispatch boundary
/// resolve descriptor refs. It retains only downstream request parameters and
/// delegates canonical tuple derivation and wire envelope assembly to Axon.
#[derive(Debug, Clone)]
pub(crate) struct LocalDaemonLoopbackInvocation {
    function_name: String,
    caller_ura: String,
    callee_ura: String,
    subject_ura: String,
    arguments: Vec<u8>,
    timeout: Duration,
    derivation_policy: InvocationDerivationPolicy,
    trace_id: Option<String>,
}

impl LocalDaemonLoopbackInvocation {
    pub(crate) fn from_target(
        function_name: &str,
        payload_json: serde_json::Value,
        caller_ura: impl Into<String>,
        callee_ura: impl Into<String>,
        subject_ura: impl Into<String>,
        derivation_policy: InvocationDerivationPolicy,
        timeout: Duration,
    ) -> anyhow::Result<Self> {
        let function_name = checked_function_name(function_name)?;
        let arguments = serde_json::to_vec(&payload_json)
            .map_err(|err| anyhow::anyhow!("encode {function_name} args: {err}"))?;
        Ok(Self {
            function_name,
            caller_ura: checked_ura(caller_ura.into(), "caller_ura")?,
            callee_ura: checked_ura(callee_ura.into(), "callee_ura")?,
            subject_ura: checked_ura(subject_ura.into(), "subject_ura")?,
            arguments,
            timeout,
            derivation_policy,
            trace_id: None,
        })
    }

    pub(crate) fn function_name(&self) -> &str {
        &self.function_name
    }

    pub(crate) fn caller_ura(&self) -> &str {
        &self.caller_ura
    }

    pub(crate) fn arguments(&self) -> &[u8] {
        &self.arguments
    }

    #[must_use]
    pub(crate) fn with_trace_id(mut self, trace_id: Option<&str>) -> Self {
        self.trace_id = trace_id
            .map(str::trim)
            .filter(|trace_id| !trace_id.is_empty())
            .map(str::to_string);
        self
    }

    pub(crate) fn invoke_request(&self) -> anyhow::Result<InvokeRequest> {
        Ok(InvokeRequest {
            envelope: Some(self.envelope()?),
            target: Some(wire_invocation_target(
                &self.function_name,
                &self.function_name,
            )?),
            arguments: self.arguments.clone(),
            content_type: "application/json".to_string(),
            timeout_seconds: self.timeout_seconds(),
            ..InvokeRequest::default()
        })
    }

    pub(crate) fn stream_request(&self) -> anyhow::Result<InvokeServerStreamRequest> {
        Ok(InvokeServerStreamRequest {
            envelope: Some(self.envelope()?),
            target: Some(wire_invocation_target(
                &self.function_name,
                &self.function_name,
            )?),
            arguments: self.arguments.clone(),
            content_type: "application/json".to_string(),
            timeout_seconds: self.timeout_seconds(),
            ..InvokeServerStreamRequest::default()
        })
    }

    pub(crate) fn envelope(&self) -> anyhow::Result<Envelope> {
        ProtoEnvelope::from_target(
            self.caller_ura.clone(),
            self.callee_ura.clone(),
            self.subject_ura.clone(),
            self.derivation_policy.clone(),
        )?
        .with_trace_id(self.trace_id.as_deref())
        .wire_envelope_for(&self.function_name, &self.arguments)
    }

    fn timeout_seconds(&self) -> i32 {
        i32::try_from(self.timeout.as_secs()).unwrap_or(i32::MAX)
    }
}

impl ProtoEnvelope {
    pub fn loopback(
        ura: impl Into<String>,
        derivation_policy: InvocationDerivationPolicy,
    ) -> anyhow::Result<Self> {
        let ura = checked_ura(ura.into(), "loopback_ura")?;
        Self::from_target(ura.clone(), ura.clone(), ura, derivation_policy)
    }

    pub fn from_target(
        caller_ura: impl Into<String>,
        callee_ura: impl Into<String>,
        subject_ura: impl Into<String>,
        derivation_policy: InvocationDerivationPolicy,
    ) -> anyhow::Result<Self> {
        let caller_ura = checked_ura(caller_ura.into(), "caller_ura")?;
        let callee_ura = checked_ura(callee_ura.into(), "callee_ura")?;
        let subject_ura = checked_ura(subject_ura.into(), "subject_ura")?;
        try_entity_ref(subject_ura.clone())?;
        let canonical = CanonicalEnvelopeBuilder::new(
            AgentIdentity::new(caller_ura, UraProfile::StrictV2),
            AgentIdentity::new(callee_ura, UraProfile::StrictV2),
            SubjectIdentity::new(subject_ura, UraProfile::StrictV2),
            derivation_policy,
        )
        .map_err(|error| anyhow::anyhow!("{error}"))?;
        Ok(Self {
            canonical,
            wire_metadata: WireEnvelopeMetadata {
                request_id: fresh_request_id(),
                ..WireEnvelopeMetadata::default()
            },
        })
    }

    pub fn federation_join_genesis(
        provisional_caller_ura: impl Into<String>,
        hub_ura: impl Into<String>,
        membership_ura: impl Into<String>,
        derivation_policy: InvocationDerivationPolicy,
    ) -> anyhow::Result<Self> {
        let caller_ura = checked_provisional_ura(provisional_caller_ura.into())?;
        let hub_ura = checked_ura(hub_ura.into(), "hub_ura")?;
        let membership_ura = checked_ura(membership_ura.into(), "membership_ura")?;

        let parsed_hub = crate::core::ura::parse_ura(&hub_ura)
            .map_err(|e| anyhow::anyhow!("hub_ura is not a valid URA: {e}"))?;
        if parsed_hub.kind != crate::core::ura::URAKind::Authority {
            anyhow::bail!("hub_ura must identify a Hub, got {:?}", parsed_hub.kind);
        }
        let parsed_membership = crate::core::ura::parse_ura(&membership_ura)
            .map_err(|e| anyhow::anyhow!("membership_ura is not a valid URA: {e}"))?;
        if parsed_membership.kind != crate::core::ura::URAKind::Device {
            anyhow::bail!(
                "membership_ura must identify a Device, got {:?}",
                parsed_membership.kind
            );
        }
        if parsed_membership.realm != parsed_hub.realm {
            anyhow::bail!(
                "membership_ura realm `{}` does not match hub realm `{}`",
                parsed_membership.realm,
                parsed_hub.realm
            );
        }
        try_entity_ref(membership_ura.clone())?;

        let canonical = CanonicalEnvelopeBuilder::new(
            AgentIdentity::new(caller_ura, UraProfile::StrictV2),
            AgentIdentity::new(hub_ura, UraProfile::StrictV2),
            SubjectIdentity::new(membership_ura, UraProfile::StrictV2),
            derivation_policy,
        )
        .map_err(|error| anyhow::anyhow!("{error}"))?;
        Ok(Self {
            canonical,
            wire_metadata: WireEnvelopeMetadata {
                request_id: fresh_request_id(),
                ..WireEnvelopeMetadata::default()
            },
        })
    }

    #[must_use]
    pub fn callee_ura(&self) -> Option<&str> {
        Some(self.canonical.callee_ura())
    }

    pub fn into_inner(self, ability: &str, arguments: &[u8]) -> anyhow::Result<Envelope> {
        self.wire_envelope_for(ability, arguments)
    }

    #[must_use]
    pub fn with_trace_id(mut self, trace_id: Option<&str>) -> Self {
        self.wire_metadata.trace_id = trace_id
            .map(str::trim)
            .filter(|trace_id| !trace_id.is_empty())
            .unwrap_or_default()
            .to_string();
        self
    }

    pub fn invoke_request(
        self,
        function_name: impl Into<String>,
        arguments: Vec<u8>,
    ) -> anyhow::Result<InvokeRequest> {
        let function_name = function_name.into();
        if function_name.trim().is_empty() {
            anyhow::bail!("function_name must not be empty");
        }
        let envelope = self.wire_envelope_for(&function_name, &arguments)?;
        Ok(InvokeRequest {
            envelope: Some(envelope),
            target: Some(wire_invocation_target(&function_name, &function_name)?),
            arguments,
            ..InvokeRequest::default()
        })
    }

    /// Sign this envelope with an explicit descriptor-bound ability ref while
    /// keeping `function_name` as the route query sent over `InvokeRequest`.
    ///
    /// Route names are caller-facing (`echo`, `fs.read`, ...); descriptor
    /// refs are control-plane facts (`<ability-ura>@<descriptor-version>`).
    /// They must not be conflated once descriptor versions are no longer
    /// defaulted.
    pub fn signed_descriptor_ref_invoke_request(
        self,
        function_name: impl Into<String>,
        descriptor_ability_ref: impl Into<String>,
        arguments: Vec<u8>,
        signer: &dyn crate::daemon::identity::self_identity::SelfIdentity,
    ) -> anyhow::Result<InvokeRequest> {
        let function_name = function_name.into();
        if function_name.trim().is_empty() {
            anyhow::bail!("function_name must not be empty");
        }
        let descriptor_ability_ref = descriptor_ability_ref.into();
        if descriptor_ability_ref.trim().is_empty() {
            anyhow::bail!("descriptor_ability_ref must not be empty");
        }
        let signed = self.sign_descriptor_bound(&descriptor_ability_ref, &arguments, signer)?;
        let envelope = signed.wire_envelope_for(&descriptor_ability_ref, &arguments)?;
        Ok(InvokeRequest {
            envelope: Some(envelope),
            target: Some(wire_invocation_target(
                &descriptor_ability_ref,
                &function_name,
            )?),
            arguments,
            ..InvokeRequest::default()
        })
    }

    /// Owner-bound asynchronous variant used by CLI and daemon relay adapters.
    ///
    /// Runtime callers receive a [`CanonicalSigner`] rather than the wider
    /// multi-owner key-service port. The request is otherwise byte-for-byte the
    /// same descriptor-bound canonical invocation as the synchronous builder.
    pub async fn signed_descriptor_ref_invoke_request_with_signer(
        mut self,
        function_name: impl Into<String>,
        descriptor_ability_ref: impl Into<String>,
        arguments: Vec<u8>,
        signer: &dyn crate::daemon::identity::self_identity::CanonicalSigner,
    ) -> anyhow::Result<InvokeRequest> {
        let function_name = function_name.into();
        if function_name.trim().is_empty() {
            anyhow::bail!("function_name must not be empty");
        }
        let descriptor_ability_ref = descriptor_ability_ref.into();
        if descriptor_ability_ref.trim().is_empty() {
            anyhow::bail!("descriptor_ability_ref must not be empty");
        }
        let descriptor = self.descriptor_bound_envelope(&descriptor_ability_ref, &arguments)?;
        let caller_ura = descriptor.envelope().caller.ura.clone();
        if signer.owner_ura() != caller_ura {
            anyhow::bail!(
                "caller signer owner mismatch: envelope caller is `{caller_ura}`, signer is `{}`",
                signer.owner_ura()
            );
        }
        let caller_signature =
            crate::daemon::invocation::caller_signature::sign_canonical_caller_signature(
                signer,
                &descriptor_bound_canonical_bytes(&descriptor),
            )
            .await
            .with_context(|| format!("sign descriptor-bound invocation as {caller_ura}"))?;
        self.wire_metadata.caller_signature = Some(caller_signature.into());
        let envelope = self.wire_envelope_for(&descriptor_ability_ref, &arguments)?;
        Ok(InvokeRequest {
            envelope: Some(envelope),
            target: Some(wire_invocation_target(
                &descriptor_ability_ref,
                &function_name,
            )?),
            arguments,
            ..InvokeRequest::default()
        })
    }

    /// Build a descriptor-bound server-stream request with the same canonical
    /// signing and metadata rules as unary invocation.
    pub async fn signed_descriptor_ref_stream_request_with_signer(
        mut self,
        function_name: impl Into<String>,
        descriptor_ability_ref: impl Into<String>,
        arguments: Vec<u8>,
        signer: &dyn crate::daemon::identity::self_identity::CanonicalSigner,
    ) -> anyhow::Result<InvokeServerStreamRequest> {
        let function_name = function_name.into();
        if function_name.trim().is_empty() {
            anyhow::bail!("function_name must not be empty");
        }
        let descriptor_ability_ref = descriptor_ability_ref.into();
        if descriptor_ability_ref.trim().is_empty() {
            anyhow::bail!("descriptor_ability_ref must not be empty");
        }
        let descriptor = self.descriptor_bound_envelope(&descriptor_ability_ref, &arguments)?;
        let caller_ura = descriptor.envelope().caller.ura.clone();
        if signer.owner_ura() != caller_ura {
            anyhow::bail!(
                "caller signer owner mismatch: envelope caller is `{caller_ura}`, signer is `{}`",
                signer.owner_ura()
            );
        }
        let caller_signature =
            crate::daemon::invocation::caller_signature::sign_canonical_caller_signature(
                signer,
                &descriptor_bound_canonical_bytes(&descriptor),
            )
            .await
            .with_context(|| format!("sign descriptor-bound invocation as {caller_ura}"))?;
        self.wire_metadata.caller_signature = Some(caller_signature.into());
        let envelope = self.wire_envelope_for(&descriptor_ability_ref, &arguments)?;
        Ok(InvokeServerStreamRequest {
            envelope: Some(envelope),
            target: Some(wire_invocation_target(
                &descriptor_ability_ref,
                &function_name,
            )?),
            arguments,
            content_type: "application/json".to_string(),
            ..InvokeServerStreamRequest::default()
        })
    }

    /// Build a descriptor-bound bidirectional session open frame with the same
    /// canonical signing and metadata rules as unary and server-stream
    /// invocation.
    pub async fn signed_descriptor_ref_bidi_open_with_signer(
        mut self,
        function_name: impl Into<String>,
        descriptor_ability_ref: impl Into<String>,
        arguments: Vec<u8>,
        signer: &dyn crate::daemon::identity::self_identity::CanonicalSigner,
    ) -> anyhow::Result<EnvelopeOpen> {
        let function_name = function_name.into();
        if function_name.trim().is_empty() {
            anyhow::bail!("function_name must not be empty");
        }
        let descriptor_ability_ref = descriptor_ability_ref.into();
        if descriptor_ability_ref.trim().is_empty() {
            anyhow::bail!("descriptor_ability_ref must not be empty");
        }
        let descriptor = self.descriptor_bound_envelope(&descriptor_ability_ref, &arguments)?;
        let caller_ura = descriptor.envelope().caller.ura.clone();
        if signer.owner_ura() != caller_ura {
            anyhow::bail!(
                "caller signer owner mismatch: envelope caller is `{caller_ura}`, signer is `{}`",
                signer.owner_ura()
            );
        }
        let caller_signature =
            crate::daemon::invocation::caller_signature::sign_canonical_caller_signature(
                signer,
                &descriptor_bound_canonical_bytes(&descriptor),
            )
            .await
            .with_context(|| format!("sign descriptor-bound invocation as {caller_ura}"))?;
        self.wire_metadata.caller_signature = Some(caller_signature.into());
        let envelope = self.wire_envelope_for(&descriptor_ability_ref, &arguments)?;
        Ok(EnvelopeOpen {
            envelope: Some(envelope),
            target: Some(wire_invocation_target(
                &descriptor_ability_ref,
                &function_name,
            )?),
            initial_args: arguments,
            args_content_type: "application/json".to_string(),
            streams: vec![StreamDescriptor {
                stream_id: 1,
                content_type: "application/json".to_string(),
                ordering: "STRICT".to_string(),
                ..StreamDescriptor::default()
            }],
            content_envelope: Some(ContentEnvelope {
                content_type: "application/json".to_string(),
                encoding: "identity".to_string(),
                ..ContentEnvelope::default()
            }),
            ..EnvelopeOpen::default()
        })
    }

    /// Attach an Ed25519 caller signature over Axon's
    /// `DescriptorBoundEnvelope::canonical_bytes()`.
    pub fn sign_descriptor_bound(
        mut self,
        ability: &str,
        arguments: &[u8],
        signer: &dyn crate::daemon::identity::self_identity::SelfIdentity,
    ) -> anyhow::Result<Self> {
        let descriptor = self.descriptor_bound_envelope(ability, arguments)?;
        let caller_ura = descriptor.envelope().caller.ura.clone();
        let public_key = signer
            .public_key(&caller_ura)
            .with_context(|| format!("resolve public signing projection for {caller_ura}"))?;
        let signature = signer
            .sign_bound(
                &caller_ura,
                &public_key,
                &descriptor_bound_canonical_bytes(&descriptor),
            )
            .with_context(|| format!("sign descriptor-bound invocation as {caller_ura}"))?;
        self.wire_metadata.caller_signature = Some(CallerSignature {
            algorithm: "ed25519".to_string(),
            signature: signature.to_bytes().to_vec(),
            key_id_hint: BASE64_STANDARD.encode(public_key.to_bytes()),
        });
        Ok(self)
    }

    fn descriptor_bound_envelope(
        &self,
        ability: &str,
        arguments: &[u8],
    ) -> anyhow::Result<DescriptorBoundEnvelope> {
        if ability.trim().is_empty() {
            anyhow::bail!("ability must not be empty");
        }
        self.canonical
            .descriptor_bound_envelope(ability, arguments)
            .map_err(|error| anyhow::anyhow!("{error}"))
    }

    fn wire_envelope_for(&self, ability: &str, arguments: &[u8]) -> anyhow::Result<Envelope> {
        if ability.trim().is_empty() {
            anyhow::bail!("ability must not be empty");
        }
        self.canonical
            .wire_envelope(ability, arguments, self.wire_metadata.clone())
            .map_err(|error| anyhow::anyhow!("{error}"))
    }
}

fn checked_function_name(function_name: &str) -> anyhow::Result<String> {
    let function_name = function_name.trim();
    if function_name.is_empty() {
        anyhow::bail!("function_name must not be empty");
    }
    Ok(function_name.to_string())
}

/// Build the protobuf target selector used by bidi `EnvelopeOpen`.
///
/// `ability_binding` is the canonical callable identity. Signed public calls
/// must pass an AbilityDescriptorRef; trusted local calls may pass a route-only
/// identity that the daemon resolves before LocalRuntime admission.
///
/// `function_name` is a distinct execution route fact. It is never substituted
/// for the descriptor binding.
pub(crate) fn wire_invocation_target(
    ability_binding: impl Into<String>,
    function_name: impl Into<String>,
) -> anyhow::Result<InvocationTarget> {
    let ability_binding = ability_binding.into();
    let ability_binding = ability_binding.trim();
    if ability_binding.is_empty() {
        anyhow::bail!("invocation ability binding must not be empty");
    }
    let function_name = checked_function_name(&function_name.into())?;
    Ok(InvocationTarget {
        typed_target: Some(invocation_target::TypedTarget::Ability(AbilityTarget {
            ability_name: ability_binding.to_string(),
            function_name,
        })),
        ..InvocationTarget::default()
    })
}

/// Extract the canonical descriptor proof carrier from a typed invocation
/// target. Legacy target fields and metadata are intentionally ignored.
pub(crate) fn ability_binding_from_invocation_target<'a>(
    surface: &str,
    target: Option<&'a InvocationTarget>,
) -> Result<&'a str, Status> {
    let target = target.ok_or_else(|| {
        Status::invalid_argument(format!(
            "{surface}: invocation requires typed descriptor target \
             InvocationTarget.typed_target"
        ))
    })?;
    let Some(invocation_target::TypedTarget::Ability(ability)) = target.typed_target.as_ref()
    else {
        return Err(Status::invalid_argument(format!(
            "{surface}: invocation requires a typed Ability target"
        )));
    };
    let binding = ability.ability_name.trim();
    if binding.is_empty() {
        return Err(Status::invalid_argument(format!(
            "{surface}: typed Ability target is missing ability_name"
        )));
    }
    Ok(binding)
}

pub(crate) fn function_name_from_invocation_target<'a>(
    surface: &str,
    target: Option<&'a InvocationTarget>,
) -> Result<&'a str, Status> {
    let target = target.ok_or_else(|| {
        Status::invalid_argument(format!(
            "{surface}: invocation requires InvocationTarget.typed_target"
        ))
    })?;
    let Some(invocation_target::TypedTarget::Ability(ability)) = target.typed_target.as_ref()
    else {
        return Err(Status::invalid_argument(format!(
            "{surface}: invocation requires a typed Ability target"
        )));
    };
    let function_name = ability.function_name.trim();
    if function_name.is_empty() {
        return Err(Status::invalid_argument(format!(
            "{surface}: typed Ability target is missing function_name"
        )));
    }
    Ok(function_name)
}

pub(crate) fn descriptor_ref_from_invocation_target(
    surface: &str,
    callee_ura: &str,
    target: Option<&InvocationTarget>,
) -> Result<String, Status> {
    let raw = ability_binding_from_invocation_target(surface, target)?;
    crate::daemon::axon_bridge::descriptor_ref::require_descriptor_ref_for_wire(callee_ura, raw)
        .map_err(|error| {
            Status::invalid_argument(format!(
                "{surface}: typed Ability target must carry a complete descriptor ref for \
                 callee `{callee_ura}`: {error}"
            ))
        })
}

fn checked_ura(ura: String, field: &str) -> anyhow::Result<String> {
    crate::core::identity::RuntimeIdentityUra::parse(ura)
        .map(crate::core::identity::RuntimeIdentityUra::into_string)
        .map_err(|error| anyhow::anyhow!("{field} {error}"))
}

fn checked_provisional_ura(ura: String) -> anyhow::Result<String> {
    let ura = ura.trim().to_string();
    let Some(digest) = ura.strip_prefix("provisional:") else {
        anyhow::bail!("provisional_caller_ura must start with `provisional:`");
    };
    if digest.len() != 64 || !digest.chars().all(|ch| ch.is_ascii_hexdigit()) {
        anyhow::bail!("provisional_caller_ura must be `provisional:` plus 64 hex characters");
    }
    Ok(ura)
}

pub(crate) fn try_entity_ref(ura: String) -> anyhow::Result<EntityRef> {
    let kind = EntityRefKindResolution::from_ura(&ura)?.protobuf_kind();
    Ok(EntityRef {
        kind: kind as i32,
        ura,
        profile: DEFAULT_URA_PROFILE.to_string(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntityRefKindResolution {
    Agent,
    Ability,
    Device,
    Resource,
    Session,
    Continuation,
    StateObject,
}

impl EntityRefKindResolution {
    fn from_ura(ura: &str) -> anyhow::Result<Self> {
        if let Some(resolution) = top_level_subject_resolution(ura) {
            return Ok(resolution);
        }
        match crate::core::ura::parse_ura(ura.trim()).map(|parsed| parsed.kind) {
            Ok(crate::core::ura::URAKind::Agent) => Ok(Self::Agent),
            Ok(crate::core::ura::URAKind::Ability) => Ok(Self::Ability),
            Ok(crate::core::ura::URAKind::Device) => Ok(Self::Device),
            Ok(crate::core::ura::URAKind::Resource) => Ok(Self::Resource),
            Ok(other) => {
                anyhow::bail!("subject_ref_kind_unsupported:{}", subject_kind_label(other))
            }
            Err(err) => anyhow::bail!("subject_ref_ura_parse_failed:{err}"),
        }
    }

    fn protobuf_kind(self) -> EntityRefKind {
        match self {
            Self::Agent => EntityRefKind::Agent,
            Self::Ability => EntityRefKind::Ability,
            Self::Device => EntityRefKind::Device,
            Self::Resource => EntityRefKind::Resource,
            Self::Session => EntityRefKind::Session,
            Self::Continuation => EntityRefKind::Continuation,
            Self::StateObject => EntityRefKind::StateObject,
        }
    }
}

fn top_level_subject_resolution(ura: &str) -> Option<EntityRefKindResolution> {
    let rest = ura.trim().strip_prefix(crate::core::ura::URA_SCHEME)?;
    let mut segments = rest.split('/');
    let first = segments.next()?;
    let (realm, role) = if first == "r" {
        (segments.next()?, segments.next()?)
    } else {
        (first, segments.next()?)
    };
    if realm.is_empty() || role.is_empty() {
        return None;
    }
    match role {
        "agent" | "agents" => Some(EntityRefKindResolution::Agent),
        "ability" | "abilities" => Some(EntityRefKindResolution::Ability),
        "device" | "devices" => Some(EntityRefKindResolution::Device),
        "resource" | "resources" => Some(EntityRefKindResolution::Resource),
        "session" | "sessions" => Some(EntityRefKindResolution::Session),
        "continuation" | "continuations" => Some(EntityRefKindResolution::Continuation),
        "state_object" | "state-object" | "state_objects" | "state-objects" | "state"
        | "states" => Some(EntityRefKindResolution::StateObject),
        _ => None,
    }
}

fn subject_kind_label(kind: crate::core::ura::URAKind) -> &'static str {
    match kind {
        crate::core::ura::URAKind::Authority => "Hub",
        crate::core::ura::URAKind::User => "User",
        crate::core::ura::URAKind::Agent => "Agent",
        crate::core::ura::URAKind::Ability => "Ability",
        crate::core::ura::URAKind::Device => "Device",
        crate::core::ura::URAKind::Resource => "Resource",
        crate::core::ura::URAKind::Unknown => "Unknown",
    }
}

fn fresh_request_id() -> String {
    let mut bytes = [0_u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("req-{}", hex::encode(bytes))
}

/// Content type the daemon dispatch surfaces emit on
/// `InvokeResponse.result` / `InvokeStreamChunk.content_type`.
/// Centralised here so call sites cannot drift away from the value
/// PR-4's baselines expect.
pub(crate) const FEDERATION_RESULT_CONTENT_TYPE: &str = "application/json";

/// Boxed pinned stream type used for both server-stream and
/// bidirectional response stream associated types.
pub(crate) type BoxedDownStream<T> =
    std::pin::Pin<Box<dyn futures::Stream<Item = Result<T, tonic::Status>> + Send + 'static>>;

/// Extract the namespace.resolve target URA from a request envelope.
///
/// Route selection is bound to the explicit callee tuple field. Caller identity
/// is authority/proof input and must never be substituted as a route target.
/// Shared by unary, server-stream, bidi, and carrier-v1 local dispatch paths.
pub(crate) fn callee_ura_from_envelope(
    envelope: Option<&Envelope>,
    label: &str,
) -> Result<String, tonic::Status> {
    use tonic::Status;

    let envelope = envelope.ok_or_else(|| {
        Status::invalid_argument(format!(
            "{label} request missing envelope for namespace.resolve"
        ))
    })?;
    let callee_ura = envelope
        .callee
        .as_ref()
        .map(|identity| identity.ura.trim())
        .filter(|ura| !ura.is_empty())
        .ok_or_else(|| {
            Status::invalid_argument(format!(
                "{label} request envelope must carry callee URA for namespace.resolve"
            ))
        })?;
    crate::core::ura::parse_ura(callee_ura)
        .map_err(|err| Status::invalid_argument(format!("{label} target URA is invalid: {err}")))?;
    Ok(callee_ura.to_string())
}

/// Map an Axon `LocalRuntime` dispatch error onto the tonic `Status`
/// the daemon wire surfaces return. Shared by the unary, server-stream,
/// and bidi local-dispatch paths.
pub(crate) fn status_from_axon_invoke_error(
    surface: &str,
    ability: &str,
    err: axon_sdk::invocation::AxonError,
) -> tonic::Status {
    use axon_sdk::invocation::AxonErrorKind;
    use tonic::Status;

    let message =
        format!("{surface}: Axon LocalRuntime dispatch of ability `{ability}` failed: {err}");
    if err.reason.contains("unknown_ability") || err.reason.contains("mode_not_supported") {
        return Status::not_found(message);
    }
    if axon_error_is_caller_trust_denial(&err) {
        let message = if message.contains("not in the realm trust anchor") {
            message
        } else {
            format!("{message}; caller is not in the realm trust anchor")
        };
        return Status::permission_denied(message);
    }
    match err.kind {
        AxonErrorKind::Cancelled => Status::cancelled(message),
        AxonErrorKind::DeadlineExceeded => Status::deadline_exceeded(message),
        AxonErrorKind::Unavailable => Status::unavailable(message),
        AxonErrorKind::InvalidArgument => Status::invalid_argument(message),
        AxonErrorKind::ResourceExhausted => Status::resource_exhausted(message),
        AxonErrorKind::PermissionDenied => Status::permission_denied(message),
        AxonErrorKind::Internal => Status::internal(message),
    }
}

fn axon_error_is_caller_trust_denial(err: &axon_sdk::invocation::AxonError) -> bool {
    use axon_sdk::invocation::ErrorCode;

    matches!(
        err.code,
        ErrorCode::CallerUnknown | ErrorCode::CallerKeyNotFound | ErrorCode::CallerKeyRevoked
    )
}

/// Explain why a descriptor-bound request cannot be routed through a
/// different dispatch identity than the ability identity signed in the
/// envelope.
///
/// Axon `LocalRuntime` intentionally resolves descriptor-bound handlers by
/// the selected canonical ability identity. EasyNet-Cli may choose
/// locality and reject a route, but it must not rewrite the signed
/// callable identity at dispatch time because that would split the
/// receipt's governed descriptor from the executed implementation.
pub(crate) fn dispatch_key_mismatch_message(
    surface: &str,
    signed_ability: &str,
    dispatch_ability: &str,
    route_ura: &str,
) -> String {
    format!(
        "{surface}: selected route `{route_ura}` resolves to dispatch identity \
         `{dispatch_ability}`, but the descriptor-bound envelope signs ability \
         `{signed_ability}`; Axon LocalRuntime can only execute the signed \
         ability identity"
    )
}

/// Status-shaped counterpart to [`dispatch_key_mismatch_message`].
pub(crate) fn status_from_dispatch_key_mismatch(
    surface: &str,
    signed_ability: &str,
    dispatch_ability: &str,
    route_ura: &str,
) -> tonic::Status {
    tonic::Status::failed_precondition(dispatch_key_mismatch_message(
        surface,
        signed_ability,
        dispatch_ability,
        route_ura,
    ))
}

/// Parse a JSON-encoded request body, mapping any error to
/// `Status::invalid_argument` with a useful message. Centralised so
/// every wrapper dispatch site reports parse failures the same way.
pub(crate) fn parse_json_args<T: serde::de::DeserializeOwned>(
    arguments: &[u8],
) -> Result<T, Status> {
    serde_json::from_slice(arguments).map_err(|err| {
        Status::invalid_argument(format!(
            "federation wrapper: failed to decode JSON arguments: {err}"
        ))
    })
}

/// Serialize product output without asserting invocation lifecycle state.
/// Exact-route providers return these bytes to Axon; only LocalRuntime may
/// project admission and terminal state onto the public Invoke response.
pub(crate) fn encode_json_payload<T: serde::Serialize>(response: &T) -> Result<Vec<u8>, Status> {
    serde_json::to_vec(response).map_err(|err| {
        Status::internal(format!(
            "federation wrapper: failed to encode JSON response: {err}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axon_sdk::pb::axon::v1::causal_context;
    use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};

    struct TestSigner(SigningKey);

    impl crate::daemon::identity::self_identity::SelfIdentity for TestSigner {
        fn sign(
            &self,
            _self_ura: &str,
            canonical_bytes: &[u8],
        ) -> Result<Signature, crate::daemon::identity::self_identity::SelfIdentityError> {
            Ok(self.0.sign(canonical_bytes))
        }

        fn public_key(
            &self,
            _self_ura: &str,
        ) -> Result<VerifyingKey, crate::daemon::identity::self_identity::SelfIdentityError>
        {
            Ok(self.0.verifying_key())
        }
    }

    #[test]
    fn caller_trust_error_codes_map_to_permission_denied_transport_status() {
        use axon_sdk::invocation::{AxonError, ErrorCode};

        for code in [
            ErrorCode::CallerUnknown,
            ErrorCode::CallerKeyNotFound,
            ErrorCode::CallerKeyRevoked,
        ] {
            let error = AxonError::invalid_argument(code.as_str())
                .with_code(code)
                .with_message("typed trust rejection");
            let status = status_from_axon_invoke_error("InvokeStream", "test.stream", error);
            assert_eq!(
                status.code(),
                tonic::Code::PermissionDenied,
                "{} must remain a permission denial at a streaming transport boundary",
                code.as_str()
            );
        }
    }

    #[test]
    fn trust_wording_cannot_reclassify_non_trust_error_codes() {
        use axon_sdk::invocation::{AxonError, ErrorCode};

        let error = AxonError::invalid_argument("caller not trusted")
            .with_code(ErrorCode::RequestPayloadInvalid)
            .with_message("realm_trust_anchor: no entry");
        let status = status_from_axon_invoke_error("InvokeStream", "test.stream", error);
        assert_eq!(status.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn fresh_root_policy_builds_full_tuple_and_nonce() {
        let hub = crate::core::ura::hub_ura("acme");
        let subject = crate::core::ura::owner_ability_ura(&hub, "federation.resolve")
            .expect("realm Authority ability subject");
        let env = ProtoEnvelope::from_target(
            "easynet:///r/acme/device/dev-a",
            &hub,
            &subject,
            InvocationDerivationPolicy::FreshRoot,
        )
        .unwrap()
        .into_inner("federation.resolve", b"{}")
        .unwrap();
        assert_eq!(env.caller.unwrap().ura, "easynet:///r/acme/device/dev-a");
        assert_eq!(env.callee.unwrap().ura, hub);
        assert_eq!(env.subject.unwrap().ura, subject);
        let subject_ref = try_entity_ref(subject).unwrap();
        assert_eq!(subject_ref.kind, EntityRefKind::Ability as i32);
        assert!(env.request_id.starts_with("req-"));
        assert_eq!(env.invocation_nonce.len(), 16);
        assert!(matches!(
            env.causal_context.and_then(|ctx| ctx.form),
            Some(causal_context::Form::None(_))
        ));
    }

    #[test]
    fn explicit_policy_preserves_caller_selected_nonce() {
        let nonce = [0x5A; 16];
        let env = ProtoEnvelope::from_target(
            "easynet:///r/acme/device/dev-a",
            "easynet:///r/acme/device/dev-b",
            "easynet:///r/acme/resource/task-1",
            InvocationDerivationPolicy::Explicit {
                invocation_nonce: nonce,
                causal_context: axon_sdk::invocation::CausalContext::None,
            },
        )
        .unwrap()
        .into_inner("task.run", b"{}")
        .unwrap();

        assert_eq!(env.invocation_nonce, nonce);
        assert!(matches!(
            env.causal_context.and_then(|ctx| ctx.form),
            Some(causal_context::Form::None(_))
        ));
    }

    #[test]
    fn canonical_envelope_ownership_stays_in_axon() {
        let source = include_str!("invocation_wire.rs");
        let nonce_generator = ["fresh", "_nonce"].concat();
        let tuple_constructor = ["InvocationEnvelope", "::from_wire_parts"].concat();
        let wire_literal = ["Envelope", " {"].concat();
        let wrapped_wire_literal = ["Ok(", &wire_literal].concat();
        let hidden_root_default = [
            "derivation_policy:",
            " InvocationDerivationPolicy::FreshRoot",
        ]
        .concat();
        let causal_policy_override = ["fn with_", "causal_context"].concat();

        assert!(!source.contains(&nonce_generator));
        assert!(!source.contains(&tuple_constructor));
        assert!(!source.contains(&hidden_root_default));
        assert!(!source.contains(&causal_policy_override));
        assert!(!source.lines().any(|line| {
            line.trim_start().starts_with(&wire_literal) || line.contains(&wrapped_wire_literal)
        }));
        assert!(source.contains("CanonicalEnvelopeBuilder"));
    }

    #[test]
    fn target_rejects_provisional_caller() {
        let provisional = format!("provisional:{}", "a".repeat(64));
        let hub = crate::core::ura::hub_ura("acme");
        let subject = "easynet:///r/acme/device/dev-a";
        let err = ProtoEnvelope::from_target(
            provisional,
            hub,
            subject,
            InvocationDerivationPolicy::FreshRoot,
        )
        .unwrap_err();
        assert!(format!("{err}").contains("caller_ura is not a valid URA"));
    }

    #[test]
    fn callee_ura_from_envelope_extracts_explicit_callee() {
        let envelope = ProtoEnvelope::from_target(
            "easynet:///r/acme/device/caller",
            "easynet:///r/acme/device/callee",
            "easynet:///r/acme/device/callee",
            InvocationDerivationPolicy::FreshRoot,
        )
        .unwrap()
        .into_inner("device.ping", b"{}")
        .unwrap();

        let callee = callee_ura_from_envelope(Some(&envelope), "Invoke").unwrap();
        assert_eq!(callee, "easynet:///r/acme/device/callee");
    }

    #[test]
    fn callee_ura_from_envelope_rejects_caller_only_tuple() {
        let envelope = Envelope {
            caller: Some(axon_sdk::pb::axon::v1::AgentIdentity {
                ura: "easynet:///r/acme/device/caller".to_string(),
                profile: DEFAULT_URA_PROFILE.to_string(),
            }),
            callee: None,
            subject: Some(axon_sdk::pb::axon::v1::SubjectIdentity {
                ura: "easynet:///r/acme/device/caller".to_string(),
                profile: DEFAULT_URA_PROFILE.to_string(),
            }),
            ..Envelope::default()
        };

        let err = callee_ura_from_envelope(Some(&envelope), "Invoke").unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(
            err.message().contains("must carry callee URA"),
            "unexpected error: {}",
            err.message()
        );
    }

    #[test]
    fn federation_join_genesis_accepts_only_provisional_join_tuple() {
        let provisional = format!("provisional:{}", "a".repeat(64));
        let hub = crate::core::ura::hub_ura("acme");
        let membership = "easynet:///r/acme/device/dev-a";
        let env = ProtoEnvelope::federation_join_genesis(
            &provisional,
            &hub,
            membership,
            InvocationDerivationPolicy::FreshRoot,
        )
        .unwrap()
        .into_inner("federation.join", b"{}")
        .unwrap();
        assert_eq!(env.caller.unwrap().ura, provisional);
        assert_eq!(env.callee.unwrap().ura, hub);
        assert_eq!(env.subject.unwrap().ura, membership);

        let cross_realm = ProtoEnvelope::federation_join_genesis(
            format!("provisional:{}", "b".repeat(64)),
            crate::core::ura::hub_ura("acme"),
            "easynet:///r/other/device/dev-a",
            InvocationDerivationPolicy::FreshRoot,
        )
        .unwrap_err();
        assert!(format!("{cross_realm}").contains("does not match hub realm"));
    }

    #[test]
    fn loopback_sets_caller_callee_subject_to_same_ura() {
        let req = ProtoEnvelope::loopback(
            "easynet:///r/acme/device/dev-a",
            InvocationDerivationPolicy::FreshRoot,
        )
        .unwrap()
        .invoke_request("federation.discover", b"{}".to_vec())
        .unwrap();
        let env = req.envelope.unwrap();
        assert_eq!(env.caller.unwrap().ura, "easynet:///r/acme/device/dev-a");
        assert_eq!(env.callee.unwrap().ura, "easynet:///r/acme/device/dev-a");
        assert_eq!(env.subject.unwrap().ura, "easynet:///r/acme/device/dev-a");
        assert_eq!(
            try_entity_ref("easynet:///r/acme/device/dev-a".to_string())
                .unwrap()
                .kind,
            EntityRefKind::Device as i32
        );
        assert_eq!(
            function_name_from_invocation_target("test invoke", req.target.as_ref()).unwrap(),
            "federation.discover"
        );
    }

    #[test]
    fn hub_and_user_subject_refs_are_rejected() {
        let hub = crate::core::ura::hub_ura("acme");
        let hub_err = ProtoEnvelope::from_target(
            "easynet:///r/acme/device/dev-a",
            &hub,
            &hub,
            InvocationDerivationPolicy::FreshRoot,
        )
        .unwrap_err();
        assert!(format!("{hub_err}").contains("subject_ref_kind_unsupported:Hub"));

        let user_err = ProtoEnvelope::from_target(
            "easynet:///r/acme/device/dev-a",
            &hub,
            "easynet:///r/acme/user/alice",
            InvocationDerivationPolicy::FreshRoot,
        )
        .unwrap_err();
        assert!(format!("{user_err}").contains("subject_ref_kind_unsupported:User"));
    }

    #[test]
    fn resource_subject_with_agent_path_segment_stays_resource() {
        let env = ProtoEnvelope::from_target(
            "easynet:///r/acme/device/dev-a",
            "easynet:///r/acme/device/dev-a",
            "easynet:///r/acme/resource/project/agent/audit-log",
            InvocationDerivationPolicy::FreshRoot,
        )
        .unwrap()
        .into_inner("resource.read", b"{}")
        .unwrap();
        let subject_ref = try_entity_ref(env.subject.unwrap().ura).unwrap();
        assert_eq!(subject_ref.kind, EntityRefKind::Resource as i32);
    }

    #[test]
    fn invocation_wire_entity_ref_kind_resolution_preserves_canonical_kinds() {
        let cases = [
            ("easynet:///r/acme/agent/alice.worker", EntityRefKind::Agent),
            (
                "easynet:///r/acme/ability/device.dev-a.observe.health",
                EntityRefKind::Ability,
            ),
            ("easynet:///r/acme/device/dev-a", EntityRefKind::Device),
            (
                "easynet:///r/acme/resource/user.alice/session/read",
                EntityRefKind::Resource,
            ),
        ];

        for (ura, expected) in cases {
            assert_eq!(
                try_entity_ref(ura.to_string()).unwrap().kind,
                expected as i32
            );
        }
    }

    #[test]
    fn invocation_wire_entity_ref_kind_resolution_preserves_top_level_subject_forms() {
        let cases = [
            ("easynet:///r/acme/session/s1", EntityRefKind::Session),
            (
                "easynet:///r/acme/continuations/c1",
                EntityRefKind::Continuation,
            ),
            (
                "easynet:///r/acme/state-objects/runtime",
                EntityRefKind::StateObject,
            ),
        ];

        for (ura, expected) in cases {
            assert_eq!(
                try_entity_ref(ura.to_string()).unwrap().kind,
                expected as i32
            );
        }
    }

    #[test]
    fn invocation_wire_entity_ref_kind_resolution_rejects_unsupported_canonical_kinds() {
        let user_err = try_entity_ref("easynet:///r/acme/user/alice".to_string()).unwrap_err();
        assert!(format!("{user_err}").contains("subject_ref_kind_unsupported:User"));

        let hub = crate::core::ura::hub_ura("acme");
        let hub_err = try_entity_ref(hub).unwrap_err();
        assert!(format!("{hub_err}").contains("subject_ref_kind_unsupported:Hub"));
    }

    #[test]
    fn invalid_ura_is_rejected_before_wire_send() {
        let err = ProtoEnvelope::loopback("agent://self", InvocationDerivationPolicy::FreshRoot)
            .unwrap_err();
        assert!(format!("{err}").contains("valid URA"));
    }

    #[test]
    fn wire_invocation_target_separates_ability_binding_and_function_name() {
        let target = wire_invocation_target(" descriptor-ref ", " demo.echo ").unwrap();
        let invocation_target::TypedTarget::Ability(ability) =
            target.typed_target.expect("typed ability target");
        assert_eq!(ability.ability_name, "descriptor-ref");
        assert_eq!(ability.function_name, "demo.echo");

        let err = wire_invocation_target("  ", "demo.echo").unwrap_err();
        assert!(format!("{err}").contains("ability binding must not be empty"));
        let err = wire_invocation_target("descriptor-ref", "  ").unwrap_err();
        assert!(format!("{err}").contains("function_name must not be empty"));
    }

    #[test]
    fn signed_request_signs_descriptor_bound_canonical_bytes() {
        let signer = TestSigner(SigningKey::from_bytes(&[0x37; 32]));
        let payload = br#"{"x":1}"#.to_vec();
        let callee = "easynet:///r/acme/device/dev-a";
        let descriptor_ref = format!(
            "{}@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
            crate::core::ura::owner_ability_ura(callee, "demo.echo").unwrap()
        );
        let envelope = ProtoEnvelope::from_target(
            "easynet:///r/acme/device/dev-a",
            callee,
            "easynet:///r/acme/device/dev-a",
            InvocationDerivationPolicy::FreshRoot,
        )
        .unwrap();
        let descriptor = envelope
            .descriptor_bound_envelope(&descriptor_ref, &payload)
            .unwrap();
        let request = envelope
            .signed_descriptor_ref_invoke_request(
                "demo.echo",
                descriptor_ref.clone(),
                payload,
                &signer,
            )
            .unwrap();
        assert_eq!(
            descriptor_ref_from_invocation_target(
                "test signed request",
                callee,
                request.target.as_ref(),
            )
            .unwrap(),
            descriptor_ref
        );
        assert!(request.metadata.is_empty());
        let signature = request
            .envelope
            .unwrap()
            .caller_signature
            .expect("signed request carries caller_signature");
        let signature_bytes: [u8; 64] = signature.signature.as_slice().try_into().unwrap();
        signer
            .0
            .verifying_key()
            .verify(
                &descriptor_bound_canonical_bytes(&descriptor),
                &Signature::from_bytes(&signature_bytes),
            )
            .expect("signature must verify against descriptor-bound canonical bytes");
    }

    #[test]
    fn descriptor_ref_signed_request_keeps_route_name_separate_from_signature_target() {
        let signer = TestSigner(SigningKey::from_bytes(&[0x38; 32]));
        let payload = br#"{"message":"hi"}"#.to_vec();
        let callee = "easynet:///r/acme/device/dev-a";
        let descriptor_ref = format!(
            "{}@2.3.0#bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb!invoke",
            crate::core::ura::owner_ability_ura(callee, "echo").unwrap()
        );
        let envelope = ProtoEnvelope::from_target(
            "easynet:///r/acme/device/dev-a",
            callee,
            "easynet:///r/acme/device/dev-a",
            InvocationDerivationPolicy::FreshRoot,
        )
        .unwrap();
        let descriptor = envelope
            .descriptor_bound_envelope(&descriptor_ref, &payload)
            .unwrap();
        let request = envelope
            .signed_descriptor_ref_invoke_request("echo", descriptor_ref, payload, &signer)
            .unwrap();

        assert_eq!(
            function_name_from_invocation_target("test invoke", request.target.as_ref()).unwrap(),
            "echo"
        );
        let signature = request
            .envelope
            .unwrap()
            .caller_signature
            .expect("signed request carries caller_signature");
        let signature_bytes: [u8; 64] = signature.signature.as_slice().try_into().unwrap();
        signer
            .0
            .verifying_key()
            .verify(
                &descriptor_bound_canonical_bytes(&descriptor),
                &Signature::from_bytes(&signature_bytes),
            )
            .expect("signature must verify against explicit descriptor ref");
    }
}
