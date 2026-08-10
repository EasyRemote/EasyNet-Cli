use std::collections::HashMap;
use std::marker::PhantomData;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rand::RngCore;

use crate::daemon::axon_bridge::proof_owner::descriptor_bound_canonical_bytes;
use crate::daemon::{DaemonError, Result};

/// Complete unary Invocation submitted through `DaemonClient`.
///
/// What this type is: an inspectable SDK record for the full Axon
/// Invocation tuple plus transport metadata. It can generate unary,
/// server-stream, and bidi frame-0 requests.
///
/// What this type is not: it is not a CLI-owned canonical Invocation
/// model. Canonical bytes, admission, signatures, and receipts remain
/// owned by Axon and the daemon Invocation transport.
#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone)]
pub struct DaemonInvocation {
    caller_ura: String,
    callee_ura: String,
    descriptor_ref: String,
    subject_ura: String,
    nonce: [u8; 16],
    causal_context: axon_sdk::pb::axon::v1::CausalContext,
    args: Vec<u8>,
    content_type: String,
    metadata: HashMap<String, String>,
    caller_signature: Option<axon_sdk::pb::axon::v1::CallerSignature>,
    timeout_seconds: Option<i32>,
}

#[cfg(feature = "axon-pb")]
impl DaemonInvocation {
    /// Start building a complete Invocation under an explicit Axon-owned
    /// freshness and causal-placement policy.
    pub fn builder(
        caller_ura: impl Into<String>,
        callee_ura: impl Into<String>,
        descriptor_ref: impl Into<String>,
        subject_ura: impl Into<String>,
        derivation_policy: axon_sdk::invocation::InvocationDerivationPolicy,
    ) -> Result<DaemonInvocationBuilder> {
        DaemonInvocationBuilder::new(
            caller_ura,
            callee_ura,
            descriptor_ref,
            subject_ura,
            derivation_policy,
        )
    }

    /// Caller URA.
    pub fn caller_ura(&self) -> &str {
        &self.caller_ura
    }

    /// Callee URA.
    pub fn callee_ura(&self) -> &str {
        &self.callee_ura
    }

    /// Canonical descriptor-bound Ability ref (`ability_ura@version`).
    pub fn descriptor_ref(&self) -> &str {
        &self.descriptor_ref
    }

    /// Subject URA.
    pub fn subject_ura(&self) -> &str {
        &self.subject_ura
    }

    /// Invocation nonce.
    pub fn nonce(&self) -> [u8; 16] {
        self.nonce
    }

    /// Causal context carried in the request envelope.
    pub fn causal_context(&self) -> &axon_sdk::pb::axon::v1::CausalContext {
        &self.causal_context
    }

    /// Raw ability arguments.
    pub fn args(&self) -> &[u8] {
        &self.args
    }

    /// Request content type.
    pub fn content_type(&self) -> &str {
        &self.content_type
    }

    /// Non-axiom request metadata.
    pub fn metadata(&self) -> &HashMap<String, String> {
        &self.metadata
    }

    /// Optional caller signature carried on the envelope.
    /// Per-call timeout in seconds, when the caller set one. `None`
    /// leaves the wire field at proto default (0 = daemon default).
    pub fn timeout_seconds(&self) -> Option<i32> {
        self.timeout_seconds
    }

    pub fn caller_signature(&self) -> Option<&axon_sdk::pb::axon::v1::CallerSignature> {
        self.caller_signature.as_ref()
    }

    fn signed_envelope(&self) -> Result<axon_sdk::pb::axon::v1::Envelope> {
        let caller_signature = self.caller_signature.clone().ok_or_else(|| {
            DaemonError::InvalidInvocation(
                "wire submission requires the SignedInvocation state".to_string(),
            )
        })?;
        if caller_signature.algorithm.trim().is_empty() || caller_signature.signature.is_empty() {
            return Err(DaemonError::InvalidInvocation(
                "wire submission requires complete caller signature material".to_string(),
            ));
        }
        let mut envelope = crate::daemon::invocation::ProtoEnvelope::from_target(
            self.caller_ura.clone(),
            self.callee_ura.clone(),
            self.subject_ura.clone(),
            crate::daemon::invocation::InvocationDerivationPolicy::try_explicit_from_wire_causal_context(
                self.nonce,
                self.causal_context.clone(),
            )
            .expect("DaemonInvocation builder validates the wire causal context"),
        )
        .expect("DaemonInvocation builder validates caller/callee/subject URAs")
        .into_inner(&self.descriptor_ref, &self.args)
        .expect("DaemonInvocation builder validates the complete canonical tuple");
        envelope.caller_signature = Some(caller_signature);
        Ok(envelope)
    }

    fn content_envelope(&self) -> axon_sdk::pb::axon::v1::ContentEnvelope {
        axon_sdk::pb::axon::v1::ContentEnvelope {
            content_type: self.content_type.clone(),
            encoding: "identity".to_string(),
            ..axon_sdk::pb::axon::v1::ContentEnvelope::default()
        }
    }

    fn function_name(&self) -> Result<String> {
        let ability_ura =
            axon_sdk::invocation::ability_ura_from_descriptor_ref(&self.descriptor_ref)
                .map_err(|error| DaemonError::InvalidInvocation(error.to_string()))?;
        axon_sdk::ura::public_ability_name_from_ability_ura(&self.callee_ura, ability_ura)
            .ok_or_else(|| {
                DaemonError::InvalidInvocation(format!(
                    "descriptor_ref `{}` has no public function name for callee `{}`",
                    self.descriptor_ref, self.callee_ura
                ))
            })
    }

    pub(crate) fn into_request(self) -> Result<axon_sdk::pb::axon::v1::InvokeRequest> {
        use axon_sdk::pb::axon::v1::InvokeRequest;
        let function_name = self.function_name()?;
        let target = crate::daemon::invocation::dispatch::invocation_wire::wire_invocation_target(
            &self.descriptor_ref,
            &function_name,
        )
        .map_err(|error| DaemonError::InvalidInvocation(error.to_string()))?;
        let envelope = self.signed_envelope()?;
        let content_envelope = self.content_envelope();
        Ok(InvokeRequest {
            envelope: Some(envelope),
            target: Some(target),
            arguments: self.args,
            content_type: self.content_type,
            metadata: self.metadata,
            content_envelope: Some(content_envelope),
            timeout_seconds: self.timeout_seconds.unwrap_or(0),
            ..InvokeRequest::default()
        })
    }

    pub(crate) fn into_draft(self) -> InvocationDraft {
        InvocationDraft { invocation: self }
    }

    pub(crate) fn into_server_stream_request(
        self,
    ) -> Result<axon_sdk::pb::axon::v1::InvokeServerStreamRequest> {
        use axon_sdk::pb::axon::v1::InvokeServerStreamRequest;
        let function_name = self.function_name()?;
        let target = crate::daemon::invocation::dispatch::invocation_wire::wire_invocation_target(
            &self.descriptor_ref,
            &function_name,
        )
        .map_err(|error| DaemonError::InvalidInvocation(error.to_string()))?;
        let envelope = self.signed_envelope()?;
        let content_envelope = self.content_envelope();
        Ok(InvokeServerStreamRequest {
            envelope: Some(envelope),
            target: Some(target),
            arguments: self.args,
            content_type: self.content_type,
            metadata: self.metadata,
            content_envelope: Some(content_envelope),
            ..InvokeServerStreamRequest::default()
        })
    }

    pub(crate) fn into_bidi_open_frame(
        self,
        streams: Vec<axon_sdk::pb::axon::v1::StreamDescriptor>,
    ) -> Result<axon_sdk::pb::axon::v1::InvokeBidiUp> {
        use axon_sdk::pb::axon::v1::{invoke_bidi_up, EnvelopeOpen, InvokeBidiUp};
        if streams.is_empty() {
            return Err(DaemonError::InvalidInvocation(
                "bidi streams must not be empty".to_string(),
            ));
        }
        validate_bidi_streams(&streams)?;
        let function_name = self.function_name()?;
        let target = crate::daemon::invocation::dispatch::invocation_wire::wire_invocation_target(
            &self.descriptor_ref,
            &function_name,
        )
        .map_err(|err| DaemonError::InvalidInvocation(err.to_string()))?;
        let envelope = self.signed_envelope()?;
        let content_envelope = self.content_envelope();
        let mac = envelope
            .caller_signature
            .as_ref()
            .expect("signed_envelope guarantees caller signature")
            .signature
            .clone();
        Ok(InvokeBidiUp {
            sequence: 0,
            mac,
            payload: Some(invoke_bidi_up::Payload::EnvelopeOpen(EnvelopeOpen {
                envelope: Some(envelope),
                target: Some(target),
                initial_args: self.args,
                args_content_type: self.content_type.clone(),
                streams,
                metadata: self.metadata,
                content_envelope: Some(content_envelope),
                // No session-resume semantics on the SDK frame-0 path;
                // the session extension is the transport supervisor's
                // concern (proto invoke.proto SessionOpenExt).
                session_ext: None,
            })),
        })
    }
}

/// Builder for `DaemonInvocation`.
///
/// The generic state records whether the caller has explicitly supplied the
/// Invocation arguments. A builder with `InvocationArgsUnset` cannot be
/// inspected or built, so public ingress cannot silently substitute an empty
/// payload.
#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone)]
pub struct DaemonInvocationBuilder<ArgsState = InvocationArgsUnset> {
    caller_ura: String,
    callee_ura: String,
    descriptor_ref: String,
    subject_ura: String,
    nonce: [u8; 16],
    causal_context: axon_sdk::pb::axon::v1::CausalContext,
    args: Vec<u8>,
    content_type: String,
    metadata: HashMap<String, String>,
    caller_signature: Option<axon_sdk::pb::axon::v1::CallerSignature>,
    timeout_seconds: Option<i32>,
    args_state: PhantomData<ArgsState>,
}

/// Invocation builder state before arguments are explicitly supplied.
#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, Copy)]
pub struct InvocationArgsUnset;

/// Invocation builder state after arguments are explicitly supplied.
#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, Copy)]
pub struct InvocationArgsSet;

#[cfg(feature = "axon-pb")]
impl DaemonInvocationBuilder<InvocationArgsUnset> {
    fn new(
        caller_ura: impl Into<String>,
        callee_ura: impl Into<String>,
        descriptor_ref: impl Into<String>,
        subject_ura: impl Into<String>,
        derivation_policy: axon_sdk::invocation::InvocationDerivationPolicy,
    ) -> Result<Self> {
        let caller_ura = checked_ura(caller_ura.into(), "caller_ura")?;
        let callee_ura = checked_ura(callee_ura.into(), "callee_ura")?;
        let subject_ura = checked_ura(subject_ura.into(), "subject_ura")?;
        crate::daemon::invocation::dispatch::invocation_wire::try_entity_ref(subject_ura.clone())
            .map_err(|err| {
            DaemonError::InvalidInvocation(format!("subject_ura must be descriptor-bound: {err}"))
        })?;
        let descriptor_ref = descriptor_ref.into();
        if descriptor_ref.trim().is_empty() {
            return Err(DaemonError::InvalidInvocation(
                "descriptor_ref must not be empty".to_string(),
            ));
        }
        let descriptor_ref =
            crate::daemon::axon_bridge::descriptor_ref::require_descriptor_ref_for_wire(
                &callee_ura,
                &descriptor_ref,
            )
            .map_err(|err| DaemonError::InvalidInvocation(err.to_string()))?;
        let derived = crate::daemon::invocation::ProtoEnvelope::from_target(
            caller_ura.clone(),
            callee_ura.clone(),
            subject_ura.clone(),
            derivation_policy,
        )
        .and_then(|envelope| envelope.into_inner(&descriptor_ref, &[]))
        .map_err(|error| DaemonError::InvalidInvocation(error.to_string()))?;
        let nonce = axon_sdk::invocation::try_invocation_nonce(derived.invocation_nonce)
            .map_err(|error| DaemonError::InvalidInvocation(error.to_string()))?;
        let causal_context = derived.causal_context.ok_or_else(|| {
            DaemonError::InvalidInvocation(
                "Axon derivation policy omitted causal_context".to_string(),
            )
        })?;
        Ok(Self {
            caller_ura,
            callee_ura,
            descriptor_ref,
            subject_ura,
            nonce,
            causal_context,
            args: Vec::new(),
            content_type: "application/json".to_string(),
            metadata: HashMap::new(),
            caller_signature: None,
            timeout_seconds: None,
            args_state: PhantomData,
        })
    }
}

#[cfg(feature = "axon-pb")]
impl<ArgsState> DaemonInvocationBuilder<ArgsState> {
    /// Supply raw argument bytes and content type.
    pub fn args_bytes(
        self,
        args: impl Into<Vec<u8>>,
        content_type: impl Into<String>,
    ) -> Result<DaemonInvocationBuilder<InvocationArgsSet>> {
        let content_type = content_type.into();
        if content_type.trim().is_empty() {
            return Err(DaemonError::InvalidInvocation(
                "content_type must not be empty".to_string(),
            ));
        }
        Ok(self.with_explicit_args(args.into(), content_type.trim().to_string()))
    }

    /// Supply JSON arguments.
    pub fn args_json(
        self,
        value: &serde_json::Value,
    ) -> Result<DaemonInvocationBuilder<InvocationArgsSet>> {
        let args = serde_json::to_vec(value).map_err(DaemonError::EncodeArguments)?;
        Ok(self.with_explicit_args(args, "application/json".to_string()))
    }

    fn with_explicit_args(
        self,
        args: Vec<u8>,
        content_type: String,
    ) -> DaemonInvocationBuilder<InvocationArgsSet> {
        DaemonInvocationBuilder {
            caller_ura: self.caller_ura,
            callee_ura: self.callee_ura,
            descriptor_ref: self.descriptor_ref,
            subject_ura: self.subject_ura,
            nonce: self.nonce,
            causal_context: self.causal_context,
            args,
            content_type,
            metadata: self.metadata,
            caller_signature: self.caller_signature,
            timeout_seconds: self.timeout_seconds,
            args_state: PhantomData,
        }
    }

    /// Replace non-axiom request metadata. Metadata is transported
    /// on unary/server-stream requests and on bidi frame-0
    /// `EnvelopeOpen`; it is deliberately not part of canonical
    /// invocation bytes.
    pub fn metadata(mut self, metadata: HashMap<String, String>) -> Self {
        self.metadata = metadata;
        self
    }

    /// Attach a caller signature to the envelope. The daemon
    /// admission gate verifies this against Axon canonical bytes;
    /// this SDK surface only carries the already-produced signature.
    pub fn caller_signature(
        mut self,
        caller_signature: axon_sdk::pb::axon::v1::CallerSignature,
    ) -> Self {
        self.caller_signature = Some(caller_signature);
        self
    }

    /// Per-call timeout in seconds (`InvokeRequest.timeout_seconds`,
    /// capped daemon-side by the envelope deadline). Rejects
    /// non-positive values - leave unset for the daemon default.
    pub fn timeout_seconds(mut self, seconds: i32) -> Result<Self> {
        if seconds <= 0 {
            return Err(DaemonError::InvalidInvocation(
                "timeout_seconds must be positive; omit it for the daemon default".to_string(),
            ));
        }
        self.timeout_seconds = Some(seconds);
        Ok(self)
    }
}

#[cfg(feature = "axon-pb")]
impl DaemonInvocationBuilder<InvocationArgsSet> {
    /// Inspect the current immutable draft. SDK-stable call paths use
    /// this instead of submitting the mutable builder directly.
    ///
    /// Invariant 1: the seven-tuple fields and args payload are
    /// complete before canonical prepare.
    /// Invariant 2: nonce and causal context derived by the caller-selected
    /// policy are visible in the returned `InvocationDraft`.
    pub fn inspect(&self) -> Result<InvocationDraft> {
        Ok(InvocationDraft {
            invocation: self.to_invocation(),
        })
    }

    /// Finish the SDK-stable immutable draft.
    pub fn build_draft(self) -> Result<InvocationDraft> {
        Ok(InvocationDraft {
            invocation: self.into_invocation(),
        })
    }

    /// Finish building the Invocation.
    pub fn build(self) -> DaemonInvocation {
        self.into_invocation()
    }

    fn to_invocation(&self) -> DaemonInvocation {
        DaemonInvocation {
            caller_ura: self.caller_ura.clone(),
            callee_ura: self.callee_ura.clone(),
            descriptor_ref: self.descriptor_ref.clone(),
            subject_ura: self.subject_ura.clone(),
            nonce: self.nonce,
            causal_context: self.causal_context.clone(),
            args: self.args.clone(),
            content_type: self.content_type.clone(),
            metadata: self.metadata.clone(),
            caller_signature: self.caller_signature.clone(),
            timeout_seconds: self.timeout_seconds,
        }
    }

    fn into_invocation(self) -> DaemonInvocation {
        DaemonInvocation {
            caller_ura: self.caller_ura,
            callee_ura: self.callee_ura,
            descriptor_ref: self.descriptor_ref,
            subject_ura: self.subject_ura,
            nonce: self.nonce,
            causal_context: self.causal_context,
            args: self.args,
            content_type: self.content_type,
            metadata: self.metadata,
            caller_signature: self.caller_signature,
            timeout_seconds: self.timeout_seconds,
        }
    }
}

/// Immutable SDK draft for a complete seven-tuple Invocation.
///
/// What this type is: the first non-mutable SDK object in the
/// invocation state machine. It owns a complete tuple snapshot and can
/// produce canonical signing material.
///
/// What this type is not: it is not signed and not submit-ready. The
/// only legal next proof-bearing state is `PreparedInvocation`.
#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone)]
pub struct InvocationDraft {
    invocation: DaemonInvocation,
}

#[cfg(feature = "axon-pb")]
impl InvocationDraft {
    /// Inspect the complete tuple without exposing Axon protobuf
    /// structures.
    pub fn inspect_tuple(&self) -> InvocationTuple {
        InvocationTuple::from_invocation(&self.invocation)
    }

    pub(crate) fn invocation(&self) -> &DaemonInvocation {
        &self.invocation
    }

    /// Prepare canonical signing material by delegating to Axon's
    /// descriptor-bound envelope helpers.
    pub fn prepare(&self, options: PrepareOptions) -> Result<PreparedInvocation> {
        let descriptor_bound = self.invocation.descriptor_bound_envelope()?;
        let canonical_bytes = descriptor_bound_canonical_bytes(&descriptor_bound);
        let canonical_hash_hex = hex::encode(axon_sdk::invocation::sha256(&canonical_bytes));
        let args_digest_hex = hex::encode(descriptor_bound.envelope().args_digest);
        let expires_at_unix_ms = unix_ms_after(options.expires_in);
        let signer_policy = options.into_signer_policy(expires_at_unix_ms)?;
        Ok(PreparedInvocation {
            draft: self.clone(),
            request_id: fresh_prepare_request_id(),
            descriptor_ref: self.invocation.descriptor_ref.clone(),
            descriptor_hash_hex: hex::encode(axon_sdk::invocation::sha256(
                self.invocation.descriptor_ref.as_bytes(),
            )),
            schema_hash_hex: None,
            canonical_hash_hex,
            expires_at_unix_ms,
            signing_material: SigningMaterial {
                canonical_bytes,
                args_digest_hex,
                nonce_base64: base64_encode(&self.invocation.nonce),
                signed_fields: vec![
                    "caller".to_string(),
                    "callee".to_string(),
                    "subject".to_string(),
                    "descriptor_ref".to_string(),
                    "args_digest".to_string(),
                    "nonce".to_string(),
                    "causal_context".to_string(),
                ],
                signer_policy,
            },
        })
    }

    pub(crate) fn into_daemon_invocation(self) -> DaemonInvocation {
        self.invocation
    }
}

#[cfg(feature = "axon-pb")]
fn fresh_prepare_request_id() -> String {
    let mut bytes = [0_u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("prep-{}", hex::encode(bytes))
}

/// Public tuple projection for SDK bindings.
#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct InvocationTuple {
    pub caller_ura: String,
    pub callee_ura: String,
    pub descriptor_ref: String,
    pub subject_ura: String,
    pub nonce_base64: String,
    pub causal_context: serde_json::Value,
    pub args_digest_hex: String,
    pub content_type: String,
    pub metadata: HashMap<String, String>,
    pub timeout_seconds: Option<i32>,
}

#[cfg(feature = "axon-pb")]
impl InvocationTuple {
    fn from_invocation(invocation: &DaemonInvocation) -> Self {
        Self {
            caller_ura: invocation.caller_ura.clone(),
            callee_ura: invocation.callee_ura.clone(),
            descriptor_ref: invocation.descriptor_ref.clone(),
            subject_ura: invocation.subject_ura.clone(),
            nonce_base64: base64_encode(&invocation.nonce),
            causal_context: causal_context_json(&invocation.causal_context),
            args_digest_hex: hex::encode(axon_sdk::invocation::sha256(&invocation.args)),
            content_type: invocation.content_type.clone(),
            metadata: invocation.metadata.clone(),
            timeout_seconds: invocation.timeout_seconds,
        }
    }
}

/// Options for `InvocationDraft::prepare`.
#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone)]
pub struct PrepareOptions {
    pub expires_in: Duration,
    pub signer_id: Option<String>,
    pub policy_ref: Option<String>,
    pub provider_managed_signing: bool,
}

#[cfg(feature = "axon-pb")]
impl Default for PrepareOptions {
    fn default() -> Self {
        Self {
            expires_in: Duration::from_secs(300),
            signer_id: None,
            policy_ref: None,
            provider_managed_signing: false,
        }
    }
}

#[cfg(feature = "axon-pb")]
impl PrepareOptions {
    fn into_signer_policy(self, expires_at_unix_ms: u64) -> Result<SignerPolicy> {
        let mode = if self.provider_managed_signing {
            SignerPolicyMode::ProviderManagedSigning
        } else {
            SignerPolicyMode::CallerSigning
        };
        let signer_id = optional_prepare_policy_value(self.signer_id, "signer_id")?;
        let policy_ref = optional_prepare_policy_value(self.policy_ref, "policy_ref")?;
        if mode == SignerPolicyMode::ProviderManagedSigning {
            if signer_id.is_none() {
                return Err(DaemonError::InvalidInvocation(
                    "provider-managed prepare requires signer_id".to_string(),
                ));
            }
            if policy_ref.is_none() {
                return Err(DaemonError::InvalidInvocation(
                    "provider-managed prepare requires policy_ref".to_string(),
                ));
            }
        }
        Ok(SignerPolicy {
            mode,
            signer_id: signer_id.unwrap_or_default(),
            policy_ref: policy_ref.unwrap_or_default(),
            expires_at_unix_ms,
        })
    }
}

#[cfg(feature = "axon-pb")]
fn optional_prepare_policy_value(raw: Option<String>, field: &str) -> Result<Option<String>> {
    raw.map(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(DaemonError::InvalidInvocation(format!(
                "prepare signer policy {field} must not be blank"
            )));
        }
        Ok(trimmed.to_string())
    })
    .transpose()
}

/// Prepared canonical signing material. This object is not
/// submit-ready.
#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone)]
pub struct PreparedInvocation {
    draft: InvocationDraft,
    request_id: String,
    descriptor_ref: String,
    descriptor_hash_hex: String,
    schema_hash_hex: Option<String>,
    canonical_hash_hex: String,
    expires_at_unix_ms: u64,
    signing_material: SigningMaterial,
}

#[cfg(feature = "axon-pb")]
impl PreparedInvocation {
    pub fn draft(&self) -> &InvocationDraft {
        &self.draft
    }

    pub fn tuple(&self) -> InvocationTuple {
        self.draft.inspect_tuple()
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub fn descriptor_ref(&self) -> &str {
        &self.descriptor_ref
    }

    pub fn descriptor_hash_hex(&self) -> &str {
        &self.descriptor_hash_hex
    }

    pub fn schema_hash_hex(&self) -> Option<&str> {
        self.schema_hash_hex.as_deref()
    }

    pub fn canonical_hash_hex(&self) -> &str {
        &self.canonical_hash_hex
    }

    pub fn expires_at_unix_ms(&self) -> u64 {
        self.expires_at_unix_ms
    }

    pub fn signing_material(&self) -> &SigningMaterial {
        &self.signing_material
    }

    /// Attach a caller-produced signature and move into the only
    /// submit-ready SDK state.
    pub fn sign_with_caller_signature(
        self,
        signature: CallerSignatureMaterial,
    ) -> Result<SignedInvocation> {
        signature.validate()?;
        let policy = self.signing_material.signer_policy.clone();
        let signer_id = if policy.signer_id.trim().is_empty() {
            signature.key_id_hint.clone()
        } else {
            policy.signer_id.clone()
        };
        if signer_id.trim().is_empty() {
            return Err(DaemonError::InvalidInvocation(
                "signed invocation signer id must not be empty".to_string(),
            ));
        }
        Ok(SignedInvocation {
            prepared: self,
            signature,
            signer_id,
            policy,
        })
    }

    /// Ask a provider-managed signer to attach signature material
    /// under the prepared provider-managed signing policy.
    pub fn sign_with_provider_managed_signer<S>(self, signer: &S) -> Result<SignedInvocation>
    where
        S: ProviderManagedInvocationSigner + ?Sized,
    {
        let policy = self.signing_material.signer_policy();
        if policy.mode != SignerPolicyMode::ProviderManagedSigning {
            return Err(DaemonError::InvalidInvocation(
                "provider-managed signing requires signer policy mode provider_managed_signing"
                    .to_string(),
            ));
        }
        if policy.signer_id.trim().is_empty() {
            return Err(DaemonError::InvalidInvocation(
                "provider-managed signing requires signer policy signer_id".to_string(),
            ));
        }
        if policy.policy_ref.trim().is_empty() {
            return Err(DaemonError::InvalidInvocation(
                "provider-managed signing requires signer policy policy_ref".to_string(),
            ));
        }
        let signature = signer.sign_provider_managed_invocation(&self)?;
        self.sign_with_caller_signature(signature)
    }

    pub async fn sign_with_canonical_signer(
        self,
        signer: &dyn crate::daemon::identity::self_identity::CanonicalSigner,
    ) -> Result<SignedInvocation> {
        let caller_ura = self.tuple().caller_ura;
        if signer.owner_ura() != caller_ura {
            return Err(DaemonError::InvalidInvocation(format!(
                "canonical signer owner `{}` does not match invocation caller `{caller_ura}`",
                signer.owner_ura()
            )));
        }
        let signature =
            crate::daemon::invocation::caller_signature::sign_canonical_caller_signature(
                signer,
                self.signing_material().canonical_bytes(),
            )
            .await
            .map_err(|error| {
                DaemonError::InvalidInvocation(format!(
                    "canonical invocation signing failed: {error}"
                ))
            })?;
        self.sign_with_caller_signature(CallerSignatureMaterial::new(
            signature.algorithm,
            signature.signature,
            signature.key_id_hint,
        ))
    }
}

/// Provider-managed signer seam for the `Prepared -> Signed`
/// provider-managed transition.
#[cfg(feature = "axon-pb")]
pub trait ProviderManagedInvocationSigner {
    fn sign_provider_managed_invocation(
        &self,
        prepared: &PreparedInvocation,
    ) -> Result<CallerSignatureMaterial>;
}

/// Minimal managed-signing provider boundary. Production uses the daemon UDS
/// client; an in-memory provider is permitted only in unit tests so signing
/// policy can be verified without starting a process.
#[cfg(feature = "axon-pb")]
pub trait ManagedSigningKeyService: Send + Sync {
    fn public_key(
        &self,
        key_id: &str,
    ) -> std::result::Result<
        crate::daemon::keyring::ManagedSigningKeyProjection,
        crate::daemon::identity::self_identity::SelfIdentityError,
    >;

    fn sign(
        &self,
        projection: &crate::daemon::keyring::ManagedSigningKeyProjection,
        canonical_bytes: &[u8],
    ) -> std::result::Result<
        ed25519_dalek::Signature,
        crate::daemon::identity::self_identity::SelfIdentityError,
    >;
}

#[cfg(feature = "axon-pb")]
impl ManagedSigningKeyService for crate::daemon::identity::self_identity::KeyringClient {
    fn public_key(
        &self,
        key_id: &str,
    ) -> std::result::Result<
        crate::daemon::keyring::ManagedSigningKeyProjection,
        crate::daemon::identity::self_identity::SelfIdentityError,
    > {
        self.inventory_public_key(key_id)
    }

    fn sign(
        &self,
        projection: &crate::daemon::keyring::ManagedSigningKeyProjection,
        canonical_bytes: &[u8],
    ) -> std::result::Result<
        ed25519_dalek::Signature,
        crate::daemon::identity::self_identity::SelfIdentityError,
    > {
        self.inventory_sign_bound(projection, canonical_bytes)
    }
}

/// Daemon-key-service-backed local signer. It validates the daemon-issued
/// public projection before asking the same daemon service to sign the
/// canonical material already produced by `PreparedInvocation`.
///
/// This object deliberately owns no vault path, master key, seed, or inventory
/// record. The UDS service is the only private-key custody boundary.
#[cfg(feature = "axon-pb")]
pub struct KeyServiceProviderManagedInvocationSigner {
    key_service: std::sync::Arc<dyn ManagedSigningKeyService>,
}

#[cfg(feature = "axon-pb")]
impl KeyServiceProviderManagedInvocationSigner {
    pub fn new(key_service: std::sync::Arc<dyn ManagedSigningKeyService>) -> Self {
        Self { key_service }
    }

    pub fn at_default_endpoint() -> Self {
        Self::new(std::sync::Arc::new(
            crate::daemon::identity::self_identity::KeyringClient::default_path(),
        ))
    }
}

#[cfg(feature = "axon-pb")]
impl ProviderManagedInvocationSigner for KeyServiceProviderManagedInvocationSigner {
    fn sign_provider_managed_invocation(
        &self,
        prepared: &PreparedInvocation,
    ) -> Result<CallerSignatureMaterial> {
        let policy = prepared.signing_material().signer_policy();
        let key_id = policy.signer_id.strip_prefix("signer-").ok_or_else(|| {
            DaemonError::InvalidInvocation(
                "provider-managed signer_id must use signer-{key_id}".to_string(),
            )
        })?;
        let tuple = prepared.tuple();
        let entry = self.key_service.public_key(key_id).map_err(|err| {
            DaemonError::InvalidInvocation(format!(
                "provider-managed signer key service could not resolve managed key: {err}"
            ))
        })?;
        if entry.status != crate::daemon::keyring::ManagedSigningStatus::Active {
            return Err(DaemonError::InvalidInvocation(
                "provider-managed signer key must be active".to_string(),
            ));
        }
        if entry.bound_subject.as_deref() != Some(tuple.caller_ura.as_str()) {
            return Err(DaemonError::InvalidInvocation(
                "provider-managed signer key owner does not match invocation caller".to_string(),
            ));
        }
        if entry.signer_policy_ref.as_deref() != Some(policy.policy_ref.as_str()) {
            return Err(DaemonError::InvalidInvocation(
                "provider-managed signer policy_ref does not match provider-issued key policy"
                    .to_string(),
            ));
        }
        let signature = self
            .key_service
            .sign(&entry, prepared.signing_material().canonical_bytes())
            .map_err(|err| {
                DaemonError::InvalidInvocation(format!("provider-managed signing failed: {err}"))
            })?;
        Ok(CallerSignatureMaterial::new(
            "ed25519",
            signature.to_vec(),
            policy.signer_id.clone(),
        ))
    }
}

/// Signer-facing canonical material.
#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SigningMaterial {
    canonical_bytes: Vec<u8>,
    args_digest_hex: String,
    nonce_base64: String,
    signed_fields: Vec<String>,
    signer_policy: SignerPolicy,
}

#[cfg(feature = "axon-pb")]
impl SigningMaterial {
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical_bytes
    }

    pub fn canonical_bytes_base64(&self) -> String {
        base64_encode(&self.canonical_bytes)
    }

    pub fn args_digest_hex(&self) -> &str {
        &self.args_digest_hex
    }

    pub fn nonce_base64(&self) -> &str {
        &self.nonce_base64
    }

    pub fn signed_fields(&self) -> &[String] {
        &self.signed_fields
    }

    pub fn signer_policy(&self) -> &SignerPolicy {
        &self.signer_policy
    }
}

/// Signing policy attached to prepared material.
#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SignerPolicy {
    pub mode: SignerPolicyMode,
    pub signer_id: String,
    pub policy_ref: String,
    pub expires_at_unix_ms: u64,
}

/// Signer-policy mode.
#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SignerPolicyMode {
    CallerSigning,
    ProviderManagedSigning,
}

#[cfg(feature = "axon-pb")]
impl SignerPolicyMode {
    pub fn as_str(self) -> &'static str {
        match self {
            SignerPolicyMode::CallerSigning => "caller_signing",
            SignerPolicyMode::ProviderManagedSigning => "provider_managed_signing",
        }
    }
}

/// Caller signature DTO used by SDK bindings.
#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CallerSignatureMaterial {
    pub algorithm: String,
    pub signature: Vec<u8>,
    pub key_id_hint: String,
}

#[cfg(feature = "axon-pb")]
impl CallerSignatureMaterial {
    pub fn new(
        algorithm: impl Into<String>,
        signature: impl Into<Vec<u8>>,
        key_id_hint: impl Into<String>,
    ) -> Self {
        Self {
            algorithm: algorithm.into(),
            signature: signature.into(),
            key_id_hint: key_id_hint.into(),
        }
    }

    fn validate(&self) -> Result<()> {
        if self.algorithm.trim().is_empty() {
            return Err(DaemonError::InvalidInvocation(
                "caller signature algorithm must not be empty".to_string(),
            ));
        }
        if self.signature.is_empty() {
            return Err(DaemonError::InvalidInvocation(
                "caller signature bytes must not be empty".to_string(),
            ));
        }
        Ok(())
    }

    fn into_wire(self) -> axon_sdk::pb::axon::v1::CallerSignature {
        axon_sdk::pb::axon::v1::CallerSignature {
            algorithm: self.algorithm,
            signature: self.signature,
            key_id_hint: self.key_id_hint,
        }
    }
}

/// Submit-ready immutable Invocation object.
#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone)]
pub struct SignedInvocation {
    prepared: PreparedInvocation,
    signature: CallerSignatureMaterial,
    signer_id: String,
    policy: SignerPolicy,
}

#[cfg(feature = "axon-pb")]
impl SignedInvocation {
    pub fn prepared(&self) -> &PreparedInvocation {
        &self.prepared
    }

    pub fn signature(&self) -> &CallerSignatureMaterial {
        &self.signature
    }

    pub fn signer_id(&self) -> &str {
        &self.signer_id
    }

    pub fn policy(&self) -> &SignerPolicy {
        &self.policy
    }

    pub(crate) fn into_daemon_invocation(self) -> DaemonInvocation {
        let mut invocation = self.prepared.draft.into_daemon_invocation();
        invocation.caller_signature = Some(self.signature.into_wire());
        invocation
    }

    pub(crate) fn prepare_cancel_command(&self, reason: String) -> Result<PreparedInvocation> {
        let target = self.prepared.tuple();
        let command =
            crate::daemon::invocation::dispatch::cancellation::InvocationCancelCommand::new(
                self.prepared.canonical_hash_hex(),
                None,
                reason,
            )
            .map_err(|error| DaemonError::InvalidInvocation(error.to_string()))?;
        let descriptor_ref =
            crate::daemon::axon_bridge::descriptor_ref::system_protocol_descriptor_ref_for_wire(
                &target.callee_ura,
                crate::daemon::invocation::dispatch::cancellation::ABILITY_INVOCATION_CANCEL,
                crate::daemon::ability::CallMode::Rpc,
            )
            .map_err(|error| DaemonError::InvalidInvocation(error.to_string()))?;
        DaemonInvocation::builder(
            &target.caller_ura,
            &target.callee_ura,
            descriptor_ref,
            &target.subject_ura,
            crate::daemon::invocation::RootInvocationDerivationIssuer::fresh_root(),
        )?
        .args_json(&serde_json::to_value(command).map_err(DaemonError::EncodeArguments)?)?
        .build_draft()?
        .prepare(PrepareOptions {
            expires_in: Duration::from_secs(60),
            signer_id: Some(target.caller_ura),
            policy_ref: Some("invocation.cancel.caller".to_string()),
            provider_managed_signing: false,
        })
    }
}

#[cfg(feature = "axon-pb")]
impl DaemonInvocation {
    fn descriptor_bound_envelope(&self) -> Result<axon_sdk::invocation::DescriptorBoundEnvelope> {
        let descriptor_ref =
            axon_sdk::invocation::canonical_ability_descriptor_ref(&self.descriptor_ref)
                .map_err(|err| DaemonError::InvalidInvocation(err.to_string()))?;
        let derivation_policy =
            axon_sdk::invocation::InvocationDerivationPolicy::try_explicit_from_wire_causal_context(
                self.nonce,
                self.causal_context.clone(),
            )
            .map_err(|err| DaemonError::InvalidInvocation(err.to_string()))?;
        axon_sdk::invocation::CanonicalEnvelopeBuilder::new(
            axon_sdk::invocation::AgentIdentity::new(
                &self.caller_ura,
                axon_sdk::invocation::UraProfile::StrictV2,
            ),
            axon_sdk::invocation::AgentIdentity::new(
                &self.callee_ura,
                axon_sdk::invocation::UraProfile::StrictV2,
            ),
            axon_sdk::invocation::SubjectIdentity::new(
                &self.subject_ura,
                axon_sdk::invocation::UraProfile::StrictV2,
            ),
            derivation_policy,
        )
        .and_then(|builder| builder.descriptor_bound_envelope(descriptor_ref, &self.args))
        .map_err(|err| DaemonError::InvalidInvocation(err.to_string()))
    }
}

#[cfg(feature = "axon-pb")]
fn validate_bidi_streams(streams: &[axon_sdk::pb::axon::v1::StreamDescriptor]) -> Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    if streams.len() > 1 && streams.iter().any(|stream| stream.stream_id == 0) {
        return Err(DaemonError::InvalidInvocation(
            "bidi stream_id 0 is legal only for a single stream".to_string(),
        ));
    }
    for stream in streams {
        if stream.content_type.trim().is_empty() {
            return Err(DaemonError::InvalidInvocation(
                "bidi stream content_type must not be empty".to_string(),
            ));
        }
        if stream.ordering.trim().is_empty() || stream.ordering != "STRICT" {
            return Err(DaemonError::InvalidInvocation(
                "bidi stream ordering must be STRICT".to_string(),
            ));
        }
        if !seen.insert(stream.stream_id) {
            return Err(DaemonError::InvalidInvocation(format!(
                "duplicate bidi stream_id {}",
                stream.stream_id
            )));
        }
    }
    Ok(())
}

#[cfg(feature = "axon-pb")]
fn checked_ura(value: String, field: &str) -> Result<String> {
    crate::core::identity::RuntimeIdentityUra::parse(value)
        .map(crate::core::identity::RuntimeIdentityUra::into_string)
        .map_err(|error| DaemonError::InvalidInvocation(format!("{field} {error}")))
}

#[cfg(feature = "axon-pb")]
fn unix_ms_after(duration: Duration) -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    now.saturating_add(duration).as_millis() as u64
}

#[cfg(feature = "axon-pb")]
fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(feature = "axon-pb")]
fn causal_context_json(context: &axon_sdk::pb::axon::v1::CausalContext) -> serde_json::Value {
    use axon_sdk::pb::axon::v1::causal_context::Form;
    match context.form.as_ref() {
        Some(Form::None(_)) => serde_json::json!({"form": "none"}),
        Some(Form::Scalar(receipt)) => serde_json::json!({
            "form": "scalar",
            "receipt_hash_hex": hex::encode(&receipt.receipt_hash),
            "receipt_ura": receipt.receipt_ura,
        }),
        Some(Form::List(list)) => serde_json::json!({
            "form": "list",
            "prior": list.prior.iter().map(|receipt| serde_json::json!({
                "receipt_hash_hex": hex::encode(&receipt.receipt_hash),
                "receipt_ura": receipt.receipt_ura,
            })).collect::<Vec<_>>(),
        }),
        Some(Form::Merkle(root)) => serde_json::json!({
            "form": "merkle",
            "root_hex": hex::encode(&root.root),
            "proof_ura": root.proof_ura,
        }),
        None => serde_json::json!({"form": "invalid"}),
    }
}

#[cfg(all(test, feature = "axon-pb"))]
mod tests {
    use super::*;

    struct TestManagedSigningKeyService {
        vault: std::sync::Mutex<crate::daemon::keyring::Vault>,
    }

    impl ManagedSigningKeyService for TestManagedSigningKeyService {
        fn public_key(
            &self,
            key_id: &str,
        ) -> std::result::Result<
            crate::daemon::keyring::ManagedSigningKeyProjection,
            crate::daemon::identity::self_identity::SelfIdentityError,
        > {
            self.vault
                .lock()
                .unwrap()
                .inventory_public_key(key_id)
                .map_err(|err| {
                    crate::daemon::identity::self_identity::SelfIdentityError::Rejected {
                        kind: "test_vault".into(),
                        message: err.to_string(),
                    }
                })
        }

        fn sign(
            &self,
            projection: &crate::daemon::keyring::ManagedSigningKeyProjection,
            canonical_bytes: &[u8],
        ) -> std::result::Result<
            ed25519_dalek::Signature,
            crate::daemon::identity::self_identity::SelfIdentityError,
        > {
            let subject_ura = projection.bound_subject.as_deref().ok_or_else(|| {
                crate::daemon::identity::self_identity::SelfIdentityError::Rejected {
                    kind: "test_vault".into(),
                    message: "projection is not subject-bound".into(),
                }
            })?;
            let policy_ref = projection.signer_policy_ref.as_deref().ok_or_else(|| {
                crate::daemon::identity::self_identity::SelfIdentityError::Rejected {
                    kind: "test_vault".into(),
                    message: "projection has no policy reference".into(),
                }
            })?;
            self.vault
                .lock()
                .unwrap()
                .inventory_sign_bound(
                    &projection.key_id,
                    &projection.purpose,
                    subject_ura,
                    policy_ref,
                    canonical_bytes,
                )
                .map_err(|err| {
                    crate::daemon::identity::self_identity::SelfIdentityError::Rejected {
                        kind: "test_vault".into(),
                        message: err.to_string(),
                    }
                })
        }
    }

    fn test_managed_key_service(
        subject: &str,
    ) -> (
        std::sync::Arc<dyn ManagedSigningKeyService>,
        crate::daemon::keyring::ManagedSigningKeyProjection,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let mut vault = crate::daemon::keyring::Vault::open_or_init(
            &temp.path().join("keyring.enc"),
            &crate::daemon::keyring::MasterKeySource::Explicit("test-pass".into()),
        )
        .unwrap();
        let entry = vault
            .inventory_create("agent_signing".into(), Some(subject.to_string()))
            .unwrap();
        let service: std::sync::Arc<dyn ManagedSigningKeyService> =
            std::sync::Arc::new(TestManagedSigningKeyService {
                vault: std::sync::Mutex::new(vault),
            });
        (service, entry)
    }

    fn descriptor_ref(owner_ura: &str, public_name: &str, version: &str) -> String {
        format!(
            "{}@{}#{}!invoke",
            crate::core::ura::owner_ability_ura(owner_ura, public_name).unwrap(),
            version,
            "aa".repeat(32)
        )
    }

    fn explicit_root(
        invocation_nonce: [u8; 16],
    ) -> axon_sdk::invocation::InvocationDerivationPolicy {
        axon_sdk::invocation::InvocationDerivationPolicy::Explicit {
            invocation_nonce,
            causal_context: axon_sdk::invocation::CausalContext::None,
        }
    }

    fn test_caller_signature() -> axon_sdk::pb::axon::v1::CallerSignature {
        axon_sdk::pb::axon::v1::CallerSignature {
            algorithm: "ed25519".to_string(),
            signature: vec![7; 64],
            key_id_hint: "caller-key".to_string(),
        }
    }

    #[test]
    fn invocation_builder_keeps_complete_tuple_inspectable() {
        let hub = crate::core::ura::hub_ura("acme");
        let observe_ref = descriptor_ref(&hub, "observe.health", "2.4.0");
        let invocation = DaemonInvocation::builder(
            "easynet:///r/acme/device/dev-a",
            &hub,
            &observe_ref,
            "easynet:///r/acme/device/dev-a",
            explicit_root([0x42; 16]),
        )
        .unwrap()
        .args_json(&serde_json::json!({"ok": true}))
        .unwrap()
        .build();

        assert_eq!(invocation.caller_ura(), "easynet:///r/acme/device/dev-a");
        assert_eq!(invocation.callee_ura(), hub.as_str());
        assert_eq!(invocation.descriptor_ref(), observe_ref);
        assert_eq!(invocation.subject_ura(), "easynet:///r/acme/device/dev-a");
        assert_eq!(invocation.nonce(), [0x42; 16]);
        assert_eq!(invocation.content_type(), "application/json");
        assert!(!invocation.args().is_empty());
    }

    #[test]
    fn unsigned_draft_cannot_enter_any_wire_geometry() {
        use axon_sdk::pb::axon::v1::StreamDescriptor;

        let hub = crate::core::ura::hub_ura("acme");
        let observe_ref = descriptor_ref(&hub, "observe.health", "2.4.0");
        let invocation = DaemonInvocation::builder(
            "easynet:///r/acme/device/dev-a",
            &hub,
            observe_ref,
            "easynet:///r/acme/device/dev-a",
            explicit_root([0x43; 16]),
        )
        .unwrap()
        .args_json(&serde_json::json!({"ok": true}))
        .unwrap()
        .build();

        for error in [
            invocation.clone().into_request().unwrap_err(),
            invocation.clone().into_server_stream_request().unwrap_err(),
            invocation
                .into_bidi_open_frame(vec![StreamDescriptor {
                    stream_id: 1,
                    content_type: "application/json".to_string(),
                    ordering: "STRICT".to_string(),
                    ..StreamDescriptor::default()
                }])
                .unwrap_err(),
        ] {
            assert!(
                error.to_string().contains("SignedInvocation state"),
                "unexpected unsigned wire rejection: {error}"
            );
        }
    }

    #[test]
    fn invocation_builder_emits_complete_stream_request() {
        let hub = crate::core::ura::hub_ura("acme");
        let watch_ref = descriptor_ref(&hub, "device.watch.health", "2.4.0");
        let request = DaemonInvocation::builder(
            "easynet:///r/acme/device/dev-a",
            &hub,
            &watch_ref,
            "easynet:///r/acme/device/dev-a",
            explicit_root([0x24; 16]),
        )
        .unwrap()
        .args_json(&serde_json::json!({"interval_ms": 1000}))
        .unwrap()
        .caller_signature(test_caller_signature())
        .build()
        .into_server_stream_request()
        .unwrap();

        let envelope = request
            .envelope
            .expect("stream request must carry envelope");
        assert_eq!(
            crate::daemon::invocation::dispatch::invocation_wire::function_name_from_invocation_target(
                "test stream request",
                request.target.as_ref(),
            )
            .unwrap(),
            "device.watch.health"
        );
        assert_eq!(request.content_type, "application/json");
        assert_eq!(request.arguments, br#"{"interval_ms":1000}"#);
        assert_eq!(envelope.invocation_nonce, vec![0x24; 16]);
        assert_eq!(
            envelope.caller.expect("caller required").ura,
            "easynet:///r/acme/device/dev-a"
        );
        assert_eq!(envelope.callee.expect("callee required").ura, hub.as_str());
        assert_eq!(
            envelope.subject.expect("subject required").ura,
            "easynet:///r/acme/device/dev-a"
        );
        assert!(
            envelope.causal_context.is_some(),
            "stream request must carry causal context"
        );
        assert_eq!(
            crate::daemon::invocation::dispatch::invocation_wire::descriptor_ref_from_invocation_target(
                "test stream request",
                &hub,
                request.target.as_ref(),
            )
            .unwrap(),
            watch_ref,
            "SDK wire requests must carry the descriptor ref in the canonical typed target"
        );
    }

    #[test]
    fn invocation_builder_treats_metadata_as_non_canonical() {
        let hub = crate::core::ura::hub_ura("acme");
        let observe_ref = descriptor_ref(&hub, "observe.health", "2.4.0");
        let mut metadata = HashMap::new();
        metadata.insert("x-runtime-admission".to_string(), "value".to_string());
        let request = DaemonInvocation::builder(
            "easynet:///r/acme/device/dev-a",
            &hub,
            &observe_ref,
            "easynet:///r/acme/device/dev-a",
            axon_sdk::invocation::InvocationDerivationPolicy::FreshRoot,
        )
        .unwrap()
        .metadata(metadata)
        .args_json(&serde_json::json!({"ok": true}))
        .unwrap()
        .caller_signature(test_caller_signature())
        .build()
        .into_request()
        .expect("non-canonical metadata must remain transport-only");

        assert_eq!(request.metadata["x-runtime-admission"], "value");
        assert_eq!(
            crate::daemon::invocation::dispatch::invocation_wire::descriptor_ref_from_invocation_target(
                "test unary request",
                &hub,
                request.target.as_ref(),
            )
            .unwrap(),
            observe_ref
        );
    }

    #[test]
    fn invocation_builder_emits_complete_bidi_frame0() {
        use axon_sdk::pb::axon::v1::{invoke_bidi_up, CallerSignature, StreamDescriptor};
        let mut metadata = HashMap::new();
        metadata.insert(
            "x-easynet-test-producer".to_string(),
            "producer".to_string(),
        );
        let hub = crate::core::ura::hub_ura("acme");
        let pty_ref = descriptor_ref(&hub, "device.pty.attach", "2.4.0");

        let frame = DaemonInvocation::builder(
            "easynet:///r/acme/device/dev-a",
            &hub,
            &pty_ref,
            "easynet:///r/acme/device/dev-a",
            explicit_root([0x33; 16]),
        )
        .unwrap()
        .args_json(&serde_json::json!({"session_id": "pty-1"}))
        .unwrap()
        .metadata(metadata)
        .caller_signature(CallerSignature {
            algorithm: "ed25519".to_string(),
            signature: vec![7; 64],
            key_id_hint: "caller-key".to_string(),
        })
        .build()
        .into_bidi_open_frame(vec![StreamDescriptor {
            stream_id: 1,
            content_type: "text/pty".to_string(),
            codec_params: "raw".to_string(),
            ordering: "STRICT".to_string(),
        }])
        .unwrap();

        assert_eq!(frame.sequence, 0);
        assert_eq!(frame.mac, vec![7; 64]);
        let invoke_bidi_up::Payload::EnvelopeOpen(open) =
            frame.payload.expect("frame0 must be EnvelopeOpen")
        else {
            panic!("frame0 must carry EnvelopeOpen");
        };
        let envelope = open.envelope.expect("EnvelopeOpen must carry envelope");
        assert_eq!(
            envelope.caller.expect("caller required").ura,
            "easynet:///r/acme/device/dev-a"
        );
        let target = open.target.expect("target required");
        assert_eq!(
            crate::daemon::invocation::dispatch::invocation_wire::descriptor_ref_from_invocation_target(
                "test bidi request",
                &hub,
                Some(&target),
            )
            .unwrap(),
            pty_ref
        );
        assert_eq!(open.initial_args, br#"{"session_id":"pty-1"}"#);
        assert_eq!(open.args_content_type, "application/json");
        assert_eq!(open.metadata["x-easynet-test-producer"], "producer");
        assert_eq!(open.streams.len(), 1);
        assert_eq!(open.streams[0].stream_id, 1);
        assert_eq!(
            open.content_envelope
                .expect("content envelope required")
                .encoding,
            "identity"
        );
    }

    #[test]
    fn invocation_builder_rejects_ambiguous_bidi_stream_zero() {
        use axon_sdk::pb::axon::v1::StreamDescriptor;
        let hub = crate::core::ura::hub_ura("acme");
        let pty_ref = descriptor_ref(&hub, "device.pty.attach", "2.4.0");
        let err = DaemonInvocation::builder(
            "easynet:///r/acme/device/dev-a",
            &hub,
            &pty_ref,
            "easynet:///r/acme/device/dev-a",
            axon_sdk::invocation::InvocationDerivationPolicy::FreshRoot,
        )
        .unwrap()
        .args_bytes(Vec::new(), "application/octet-stream")
        .unwrap()
        .build()
        .into_bidi_open_frame(vec![
            StreamDescriptor {
                stream_id: 0,
                content_type: "text/pty".to_string(),
                ordering: "STRICT".to_string(),
                ..StreamDescriptor::default()
            },
            StreamDescriptor {
                stream_id: 2,
                content_type: "application/json".to_string(),
                ordering: "STRICT".to_string(),
                ..StreamDescriptor::default()
            },
        ])
        .unwrap_err();

        assert!(format!("{err}").contains("stream_id 0"));
    }

    #[test]
    fn invocation_builder_rejects_invalid_ura() {
        let hub = crate::core::ura::hub_ura("acme");
        let observe_ref = descriptor_ref(&hub, "observe.health", "2.4.0");
        let err = DaemonInvocation::builder(
            "not-a-ura",
            &hub,
            observe_ref,
            &hub,
            axon_sdk::invocation::InvocationDerivationPolicy::FreshRoot,
        )
        .unwrap_err();
        assert!(format!("{err}").contains("caller_ura"));
    }

    #[test]
    fn invocation_builder_rejects_all_zero_principal_in_every_tuple_identity() {
        let caller = "easynet:///r/acme/device/dev-a";
        let hub = crate::core::ura::hub_ura("acme");
        let subject = "easynet:///r/acme/resource/user.alice/runtime-state/read";
        let observe_ref = descriptor_ref(&hub, "observe.health", "2.4.0");
        let placeholder = "00000000-0000-0000-0000-000000000000";

        for (field, candidate_caller, candidate_callee, candidate_subject) in [
            (
                "caller_ura",
                crate::core::ura::user_ura("acme", placeholder),
                hub.clone(),
                subject.to_string(),
            ),
            (
                "callee_ura",
                caller.to_string(),
                crate::core::ura::user_ura("acme", placeholder),
                subject.to_string(),
            ),
            (
                "subject_ura",
                caller.to_string(),
                hub.clone(),
                crate::core::ura::resource_dot_ura(
                    "acme",
                    &format!("user.{placeholder}"),
                    "runtime-state/read",
                ),
            ),
        ] {
            let error = DaemonInvocation::builder(
                candidate_caller,
                candidate_callee,
                &observe_ref,
                candidate_subject,
                axon_sdk::invocation::InvocationDerivationPolicy::FreshRoot,
            )
            .expect_err("all-zero principal must fail before tuple construction");
            let message = error.to_string();
            assert!(
                message.contains(field) && message.contains("all-zero principal placeholder"),
                "wrong {field} error: {message}"
            );
        }
    }

    #[test]
    fn invocation_builder_rejects_unversioned_ability() {
        let hub = crate::core::ura::hub_ura("acme");
        let err = DaemonInvocation::builder(
            "easynet:///r/acme/device/dev-a",
            &hub,
            "observe.health",
            "easynet:///r/acme/device/dev-a",
            axon_sdk::invocation::InvocationDerivationPolicy::FreshRoot,
        )
        .unwrap_err();
        assert!(format!("{err}").contains("descriptor ref"));
    }

    #[test]
    fn sdk_draft_accepts_explicit_empty_args() {
        let hub = crate::core::ura::hub_ura("acme");
        let observe_ref = descriptor_ref(&hub, "observe.health", "2.4.0");
        let draft = DaemonInvocation::builder(
            "easynet:///r/acme/device/dev-a",
            &hub,
            &observe_ref,
            "easynet:///r/acme/device/dev-a",
            axon_sdk::invocation::InvocationDerivationPolicy::FreshRoot,
        )
        .unwrap()
        .args_bytes(Vec::new(), "application/json")
        .unwrap()
        .build_draft()
        .unwrap();

        assert!(draft.invocation().args().is_empty());
        assert_eq!(draft.invocation().content_type(), "application/json");
    }

    #[test]
    fn sdk_prepare_projects_descriptor_bound_signing_material() {
        let hub = crate::core::ura::hub_ura("acme");
        let observe_ref = descriptor_ref(&hub, "observe.health", "2.4.0");
        let draft = DaemonInvocation::builder(
            "easynet:///r/acme/device/dev-a",
            &hub,
            &observe_ref,
            "easynet:///r/acme/device/dev-a",
            explicit_root([0x11; 16]),
        )
        .unwrap()
        .args_json(&serde_json::json!({"probe": true}))
        .unwrap()
        .build_draft()
        .unwrap();

        let prepared = draft
            .prepare(PrepareOptions {
                expires_in: Duration::from_secs(60),
                signer_id: Some("browser-key".to_string()),
                policy_ref: Some("policy/local".to_string()),
                provider_managed_signing: false,
            })
            .unwrap();

        assert_eq!(prepared.descriptor_ref(), observe_ref);
        assert!(!prepared.signing_material().canonical_bytes().is_empty());
        assert_eq!(
            prepared.signing_material().nonce_base64(),
            "EREREREREREREREREREREQ=="
        );
        assert_eq!(
            prepared.signing_material().signer_policy().mode.as_str(),
            "caller_signing"
        );
        assert!(prepared
            .signing_material()
            .signed_fields()
            .contains(&"descriptor_ref".to_string()));
        assert_eq!(
            prepared.tuple().subject_ura,
            "easynet:///r/acme/device/dev-a"
        );
    }

    #[test]
    fn sdk_signed_invocation_preserves_caller_signature() {
        let hub = crate::core::ura::hub_ura("acme");
        let observe_ref = descriptor_ref(&hub, "observe.health", "2.4.0");
        let prepared = DaemonInvocation::builder(
            "easynet:///r/acme/device/dev-a",
            &hub,
            &observe_ref,
            "easynet:///r/acme/device/dev-a",
            explicit_root([0x12; 16]),
        )
        .unwrap()
        .args_json(&serde_json::json!({"probe": true}))
        .unwrap()
        .build_draft()
        .unwrap()
        .prepare(PrepareOptions::default())
        .unwrap();

        let signed = prepared
            .sign_with_caller_signature(CallerSignatureMaterial::new(
                "ed25519",
                vec![0x7a; 64],
                "caller-key",
            ))
            .unwrap();
        let invocation = signed.into_daemon_invocation();
        let signature = invocation
            .caller_signature()
            .expect("signed invocation must carry caller signature");

        assert_eq!(signature.algorithm, "ed25519");
        assert_eq!(signature.signature, vec![0x7a; 64]);
        assert_eq!(signature.key_id_hint, "caller-key");
    }

    #[test]
    fn cancel_command_is_a_new_descriptor_bound_invocation() {
        let authority = crate::core::ura::hub_ura("acme");
        let target_ref = descriptor_ref(&authority, "observe.health", "2.4.0");
        let prepared = DaemonInvocation::builder(
            "easynet:///r/acme/device/dev-a",
            &authority,
            &target_ref,
            "easynet:///r/acme/device/dev-a",
            explicit_root([0x12; 16]),
        )
        .unwrap()
        .args_json(&serde_json::json!({"probe": true}))
        .unwrap()
        .build_draft()
        .unwrap()
        .prepare(PrepareOptions::default())
        .unwrap();
        let target_hash = prepared.canonical_hash_hex().to_string();
        let target_nonce = prepared.draft().invocation.nonce();
        let signed = prepared
            .sign_with_caller_signature(CallerSignatureMaterial::new(
                "ed25519",
                vec![0x7a; 64],
                "caller-key",
            ))
            .unwrap();

        let cancel = signed
            .prepare_cancel_command("operator stop".to_string())
            .expect("prepare independent cancel command");
        let tuple = cancel.tuple();
        assert_eq!(tuple.caller_ura, "easynet:///r/acme/device/dev-a");
        assert_eq!(tuple.callee_ura, authority);
        assert_ne!(cancel.draft().invocation.nonce(), target_nonce);
        assert!(cancel.descriptor_ref().contains(
            crate::daemon::invocation::dispatch::cancellation::ABILITY_INVOCATION_CANCEL
        ));
        let command: crate::daemon::invocation::dispatch::cancellation::InvocationCancelCommand =
            serde_json::from_slice(cancel.draft().invocation.args()).expect("cancel command args");
        assert_eq!(command.target_lifecycle_hash, target_hash);
        assert_eq!(command.reason, "operator stop");
    }

    #[test]
    fn sdk_signed_invocation_preserves_signer_policy_proof() {
        let hub = crate::core::ura::hub_ura("acme");
        let observe_ref = descriptor_ref(&hub, "observe.health", "2.4.0");
        let prepared = DaemonInvocation::builder(
            "easynet:///r/acme/device/dev-a",
            &hub,
            &observe_ref,
            "easynet:///r/acme/device/dev-a",
            explicit_root([0x13; 16]),
        )
        .unwrap()
        .args_json(&serde_json::json!({"probe": true}))
        .unwrap()
        .build_draft()
        .unwrap()
        .prepare(PrepareOptions {
            expires_in: Duration::from_secs(60),
            signer_id: Some("signer-alice-key-1".to_string()),
            policy_ref: Some("provider-key-inventory:sha256:test-policy".to_string()),
            provider_managed_signing: true,
        })
        .unwrap();

        let signed = prepared
            .sign_with_caller_signature(CallerSignatureMaterial::new(
                "ed25519",
                vec![0x7b; 64],
                "caller-key",
            ))
            .unwrap();

        assert_eq!(signed.signer_id(), "signer-alice-key-1");
        assert_eq!(signed.policy().mode.as_str(), "provider_managed_signing");
        assert_eq!(
            signed.policy().policy_ref,
            "provider-key-inventory:sha256:test-policy"
        );
        assert!(signed.policy().expires_at_unix_ms > 0);
    }

    #[test]
    fn sdk_prepare_rejects_incomplete_provider_managed_policy() {
        let hub = crate::core::ura::hub_ura("acme");
        let observe_ref = descriptor_ref(&hub, "observe.health", "2.4.0");
        let draft = DaemonInvocation::builder(
            "easynet:///r/acme/device/dev-a",
            &hub,
            &observe_ref,
            "easynet:///r/acme/device/dev-a",
            explicit_root([0x17; 16]),
        )
        .unwrap()
        .args_json(&serde_json::json!({"probe": true}))
        .unwrap()
        .build_draft()
        .unwrap();

        for (label, signer_id, policy_ref, expected) in [
            ("missing signer_id", None, Some("policy/local"), "signer_id"),
            (
                "blank signer_id",
                Some("  "),
                Some("policy/local"),
                "signer_id",
            ),
            (
                "missing policy_ref",
                Some("signer-key-1"),
                None,
                "policy_ref",
            ),
            (
                "blank policy_ref",
                Some("signer-key-1"),
                Some("  "),
                "policy_ref",
            ),
        ] {
            let error = draft
                .prepare(PrepareOptions {
                    expires_in: Duration::from_secs(60),
                    signer_id: signer_id.map(str::to_string),
                    policy_ref: policy_ref.map(str::to_string),
                    provider_managed_signing: true,
                })
                .expect_err(label);
            let message = error.to_string();
            assert!(
                message.contains("provider-managed prepare")
                    || message.contains("prepare signer policy"),
                "wrong {label} error: {message}"
            );
            assert!(
                message.contains(expected),
                "wrong {label} field in error: {message}"
            );
        }
    }

    #[test]
    fn sdk_signed_invocation_rejects_missing_signer_id() {
        let hub = crate::core::ura::hub_ura("acme");
        let observe_ref = descriptor_ref(&hub, "observe.health", "2.4.0");
        let prepared = DaemonInvocation::builder(
            "easynet:///r/acme/device/dev-a",
            &hub,
            &observe_ref,
            "easynet:///r/acme/device/dev-a",
            explicit_root([0x14; 16]),
        )
        .unwrap()
        .args_json(&serde_json::json!({"probe": true}))
        .unwrap()
        .build_draft()
        .unwrap()
        .prepare(PrepareOptions::default())
        .unwrap();

        let err = prepared
            .sign_with_caller_signature(CallerSignatureMaterial::new("ed25519", vec![0x7c; 64], ""))
            .unwrap_err();

        assert!(format!("{err}").contains("signer id"));
    }

    #[test]
    fn sdk_provider_managed_signer_uses_provider_inventory() {
        let caller = "easynet:///r/acme/device/dev-a";
        let (key_service, entry) = test_managed_key_service(caller);
        let signer_id = format!("signer-{}", entry.key_id);
        let policy_ref = entry.signer_policy_ref.clone().unwrap();
        let hub = crate::core::ura::hub_ura("acme");
        let observe_ref = descriptor_ref(&hub, "observe.health", "2.4.0");
        let prepared = DaemonInvocation::builder(
            caller,
            &hub,
            &observe_ref,
            caller,
            explicit_root([0x15; 16]),
        )
        .unwrap()
        .args_json(&serde_json::json!({"probe": true}))
        .unwrap()
        .build_draft()
        .unwrap()
        .prepare(PrepareOptions {
            expires_in: Duration::from_secs(60),
            signer_id: Some(signer_id.clone()),
            policy_ref: Some(policy_ref.clone()),
            provider_managed_signing: true,
        })
        .unwrap();

        let signer = KeyServiceProviderManagedInvocationSigner::new(key_service);
        let signed = prepared.sign_with_provider_managed_signer(&signer).unwrap();

        assert_eq!(signed.signer_id(), signer_id);
        assert_eq!(signed.signature().algorithm, "ed25519");
        assert_eq!(signed.signature().signature.len(), 64);
        assert_eq!(signed.signature().key_id_hint, signer_id);
        assert_eq!(signed.policy().policy_ref, policy_ref);
        assert_eq!(signed.policy().mode.as_str(), "provider_managed_signing");
    }

    #[test]
    fn sdk_provider_managed_signer_rejects_policy_ref_mismatch() {
        let caller = "easynet:///r/acme/device/dev-a";
        let (key_service, entry) = test_managed_key_service(caller);
        let hub = crate::core::ura::hub_ura("acme");
        let observe_ref = descriptor_ref(&hub, "observe.health", "2.4.0");
        let prepared = DaemonInvocation::builder(
            caller,
            &hub,
            &observe_ref,
            caller,
            explicit_root([0x16; 16]),
        )
        .unwrap()
        .args_json(&serde_json::json!({"probe": true}))
        .unwrap()
        .build_draft()
        .unwrap()
        .prepare(PrepareOptions {
            expires_in: Duration::from_secs(60),
            signer_id: Some(format!("signer-{}", entry.key_id)),
            policy_ref: Some("provider-key-inventory:sha256:wrong".to_string()),
            provider_managed_signing: true,
        })
        .unwrap();

        let signer = KeyServiceProviderManagedInvocationSigner::new(key_service);
        let err = prepared
            .sign_with_provider_managed_signer(&signer)
            .unwrap_err();

        assert!(format!("{err}").contains("policy_ref"));
    }
}
