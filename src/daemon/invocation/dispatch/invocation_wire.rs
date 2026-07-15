// EasyNet CLI — Axon invocation wire builders
// ===========================================
//
// File: src/daemon/invocation/invocation_wire.rs
// Description: Typed construction boundary for proto InvokeRequest
//              and Envelope values emitted by the CLI/daemon.
//
// Protocol Responsibility
// -----------------------
// This module owns the outbound proto envelope shape. Callers supply
// domain URAs and JSON bytes; this builder validates URA grammar,
// installs the default URA profile, and generates a replay nonce for
// complete envelopes.
//
// Implementation Approach
// -----------------------
// Keep the API deliberately small:
//   * `caller_only` for genesis/prelude calls that are admitted by a
//     special path before the full AXIOM tuple is available.
//   * `loopback` for local daemon/hub self-calls where caller,
//     callee, and subject are the same URA.
//   * `targeted` for normal caller → callee with explicit subject.
//
// Usage Contract
// --------------
// Production call sites should not hand-build `Envelope` /
// `InvokeRequest` struct literals. Tests may still construct raw
// proto fixtures when they intentionally exercise malformed shapes.
//
// Architectural Position
// ----------------------
// This is the wire-facade counterpart to `crate::core::ura` (canonical URA
// construction/parsing) and `daemon::invocation::receipts::runtime_record` (domain invocation
// records). It does not perform admission. When a caller provides a
// `SelfIdentity`, it can attach the caller signature over Axon's
// descriptor-bound canonical bytes so production daemon IPC has one
// wire-shape construction point.

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use rand::RngCore;

use anyhow::Context as _;
use tonic::{Response, Status};

use easynet_axon::invocation::DescriptorBoundEnvelope;
use easynet_axon::pb::axon::v1::{
    causal_context, AgentIdentity, CallerSignature, CausalContext, Empty, EntityRef, EntityRefKind,
    Envelope, InvokeRequest, InvokeResponse, SubjectIdentity,
};

use crate::daemon::axon_bridge::wire_descriptor::{
    descriptor_bound_from_wire_parts, WireCallerIdentity,
};

pub const DEFAULT_URA_PROFILE: &str = "easynet-strict-v2";

pub(crate) const AUTHORITY_PROOF_METADATA_KEY: &str = "x-easynet-authority-proof";
pub(crate) const SIGNED_DESCRIPTOR_REF_METADATA_KEY: &str = "x-easynet-signed-descriptor-ref";

#[derive(Debug, Clone)]
pub struct ProtoEnvelope {
    inner: Envelope,
}

impl ProtoEnvelope {
    pub fn caller_only(caller_ura: impl Into<String>) -> anyhow::Result<Self> {
        let caller_ura = checked_ura(caller_ura.into(), "caller_ura")?;
        Ok(Self {
            inner: Envelope {
                caller: Some(agent_identity(caller_ura)),
                ..Envelope::default()
            },
        })
    }

    pub fn loopback(ura: impl Into<String>) -> anyhow::Result<Self> {
        let ura = checked_ura(ura.into(), "loopback_ura")?;
        Self::targeted(ura.clone(), ura.clone(), ura)
    }

    pub fn targeted(
        caller_ura: impl Into<String>,
        callee_ura: impl Into<String>,
        subject_ura: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let caller_ura = checked_ura(caller_ura.into(), "caller_ura")?;
        let callee_ura = checked_ura(callee_ura.into(), "callee_ura")?;
        let subject_ura = checked_ura(subject_ura.into(), "subject_ura")?;
        try_entity_ref(subject_ura.clone())?;
        Ok(Self {
            inner: Envelope {
                caller: Some(agent_identity(caller_ura)),
                callee: Some(agent_identity(callee_ura)),
                subject: Some(subject_identity(subject_ura.clone())),
                request_id: fresh_request_id(),
                invocation_nonce: fresh_invocation_nonce().to_vec(),
                causal_context: Some(root_causal_context()),
                ..Envelope::default()
            },
        })
    }

    pub fn federation_join_genesis(
        provisional_caller_ura: impl Into<String>,
        hub_ura: impl Into<String>,
        membership_ura: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let caller_ura = checked_provisional_ura(provisional_caller_ura.into())?;
        let hub_ura = checked_ura(hub_ura.into(), "hub_ura")?;
        let membership_ura = checked_ura(membership_ura.into(), "membership_ura")?;

        let parsed_hub = crate::core::ura::parse_ura(&hub_ura)
            .map_err(|e| anyhow::anyhow!("hub_ura is not a valid URA: {e}"))?;
        if parsed_hub.kind != crate::core::ura::URAKind::Hub {
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

        Ok(Self {
            inner: Envelope {
                caller: Some(agent_identity(caller_ura)),
                callee: Some(agent_identity(hub_ura)),
                subject: Some(subject_identity(membership_ura)),
                request_id: fresh_request_id(),
                invocation_nonce: fresh_invocation_nonce().to_vec(),
                causal_context: Some(root_causal_context()),
                ..Envelope::default()
            },
        })
    }

    #[must_use]
    pub fn into_inner(self) -> Envelope {
        self.inner
    }

    #[must_use]
    pub fn callee_ura(&self) -> Option<&str> {
        self.inner.callee.as_ref().map(|callee| callee.ura.as_str())
    }

    #[must_use]
    pub fn with_causal_context(
        mut self,
        causal_context: easynet_axon::pb::axon::v1::CausalContext,
    ) -> Self {
        self.inner.causal_context = Some(causal_context);
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
        Ok(InvokeRequest {
            envelope: Some(self.into_inner()),
            function_name,
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
        let mut request = InvokeRequest {
            envelope: Some(signed.into_inner()),
            function_name,
            arguments,
            ..InvokeRequest::default()
        };
        request.metadata.insert(
            SIGNED_DESCRIPTOR_REF_METADATA_KEY.to_string(),
            descriptor_ability_ref,
        );
        Ok(request)
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
                &descriptor.canonical_bytes(),
            )
            .await
            .with_context(|| format!("sign descriptor-bound invocation as {caller_ura}"))?;
        self.inner.caller_signature = Some(caller_signature);
        let mut request = InvokeRequest {
            envelope: Some(self.into_inner()),
            function_name,
            arguments,
            ..InvokeRequest::default()
        };
        request.metadata.insert(
            SIGNED_DESCRIPTOR_REF_METADATA_KEY.to_string(),
            descriptor_ability_ref,
        );
        Ok(request)
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
            .sign_bound(&caller_ura, &public_key, &descriptor.canonical_bytes())
            .with_context(|| format!("sign descriptor-bound invocation as {caller_ura}"))?;
        self.inner.caller_signature = Some(CallerSignature {
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
        descriptor_bound_from_wire_parts(
            self.inner.clone(),
            ability.to_string(),
            arguments,
            WireCallerIdentity::FromEnvelope,
        )
        .map(|wire| wire.envelope)
        .map_err(|err| anyhow::anyhow!("{err}"))
    }
}

fn checked_ura(ura: String, field: &str) -> anyhow::Result<String> {
    let ura = ura.trim().to_string();
    if ura.is_empty() {
        anyhow::bail!("{field} must not be empty");
    }
    crate::core::ura::parse_ura(&ura)
        .map_err(|e| anyhow::anyhow!("{field} is not a valid URA: {e}"))?;
    Ok(ura)
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

fn agent_identity(ura: String) -> AgentIdentity {
    AgentIdentity {
        ura,
        profile: DEFAULT_URA_PROFILE.to_string(),
    }
}

fn subject_identity(ura: String) -> SubjectIdentity {
    SubjectIdentity {
        ura,
        profile: DEFAULT_URA_PROFILE.to_string(),
    }
}

pub(crate) fn try_entity_ref(ura: String) -> anyhow::Result<EntityRef> {
    let kind = infer_entity_ref_kind(&ura)?;
    Ok(EntityRef {
        kind: kind as i32,
        ura,
        profile: DEFAULT_URA_PROFILE.to_string(),
    })
}

fn infer_entity_ref_kind(ura: &str) -> anyhow::Result<EntityRefKind> {
    if let Some(kind) = top_level_subject_entity_kind(ura) {
        return Ok(kind);
    }
    match crate::core::ura::parse_ura(ura.trim()).map(|parsed| parsed.kind) {
        Ok(crate::core::ura::URAKind::Agent) => Ok(EntityRefKind::Agent),
        Ok(crate::core::ura::URAKind::Ability) => Ok(EntityRefKind::Ability),
        Ok(crate::core::ura::URAKind::Device) => Ok(EntityRefKind::Device),
        Ok(crate::core::ura::URAKind::Resource) => Ok(EntityRefKind::Resource),
        Ok(other) => anyhow::bail!("subject_ref_kind_unsupported:{other:?}"),
        Err(err) => anyhow::bail!("subject_ref_ura_parse_failed:{err}"),
    }
}

fn top_level_subject_entity_kind(ura: &str) -> Option<EntityRefKind> {
    let rest = ura.trim().strip_prefix(crate::core::ura::URA_SCHEME)?;
    let mut segments = rest.split('/');
    let realm = segments.next()?;
    let role = segments.next()?;
    if realm.is_empty() || role.is_empty() {
        return None;
    }
    match role {
        "agent" | "agents" => Some(EntityRefKind::Agent),
        "ability" | "abilities" => Some(EntityRefKind::Ability),
        "device" | "devices" => Some(EntityRefKind::Device),
        "resource" | "resources" => Some(EntityRefKind::Resource),
        "session" | "sessions" => Some(EntityRefKind::Session),
        "continuation" | "continuations" => Some(EntityRefKind::Continuation),
        "state_object" | "state-object" | "state_objects" | "state-objects" | "state"
        | "states" => Some(EntityRefKind::StateObject),
        _ => None,
    }
}

fn fresh_invocation_nonce() -> [u8; 16] {
    let mut nonce = [0_u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    nonce
}

fn fresh_request_id() -> String {
    let mut bytes = [0_u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("req-{}", hex::encode(bytes))
}

fn root_causal_context() -> CausalContext {
    CausalContext {
        form: Some(causal_context::Form::None(Empty {})),
    }
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

/// Extract the namespace.resolve target URA from a request envelope:
/// callee first, caller as fallback (genesis preludes carry caller
/// only). Validates URA grammar before returning. Shared by the
/// unary and server-stream local-dispatch paths.
pub(crate) fn target_ura_from_envelope(
    envelope: Option<&Envelope>,
    label: &str,
) -> Result<String, tonic::Status> {
    use tonic::Status;

    let envelope = envelope.ok_or_else(|| {
        Status::invalid_argument(format!(
            "{label} request missing envelope for namespace.resolve"
        ))
    })?;
    let target_ura = envelope
        .callee
        .as_ref()
        .or(envelope.caller.as_ref())
        .map(|identity| identity.ura.trim())
        .filter(|ura| !ura.is_empty())
        .ok_or_else(|| {
            Status::invalid_argument(format!(
                "{label} request envelope must carry callee or caller URA for namespace.resolve"
            ))
        })?;
    crate::core::ura::parse_ura(target_ura)
        .map_err(|err| Status::invalid_argument(format!("{label} target URA is invalid: {err}")))?;
    Ok(target_ura.to_string())
}

/// Map an Axon `LocalRuntime` dispatch error onto the tonic `Status`
/// the daemon wire surfaces return. Shared by the unary, server-stream,
/// and bidi local-dispatch paths.
pub(crate) fn status_from_axon_invoke_error(
    surface: &str,
    ability: &str,
    err: easynet_axon::invocation::AxonError,
) -> tonic::Status {
    use easynet_axon::invocation::AxonErrorKind;
    use tonic::Status;

    let message =
        format!("{surface}: Axon LocalRuntime dispatch of ability `{ability}` failed: {err}");
    if err.reason.contains("unknown_ability") || err.reason.contains("mode_not_supported") {
        return Status::not_found(message);
    }
    if axon_error_is_trust_denial(&err) {
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

fn axon_error_is_trust_denial(err: &easynet_axon::invocation::AxonError) -> bool {
    let reason = err.reason.to_ascii_uppercase();
    let message = err.message.to_ascii_lowercase();
    reason.contains("CALLER_KEY_NOT_FOUND")
        || reason.contains("CALLER_KEY_REVOKED")
        || message.contains("realm_trust_anchor: no entry")
        || message.contains("caller not trusted")
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

/// Encode a typed federation response into `InvokeResponse.result`
/// with `result_content_type = "application/json"`. Mapping any
/// serialisation error to `Status::internal` because the wrappers
/// use serde-derived types — failure here is a programmer bug, not
/// a caller bug.
///
/// `state` is set to `INVOCATION_STATE_COMPLETED` so unary callers
/// that grep on `resp.state == "completed"` (Go-side
/// `stateString` mapping) see the expected wire-visible success
/// signal. Without this the proto default-zero value
/// (`INVOCATION_STATE_UNSPECIFIED`) collapses to `"failed"` on the
/// Go side per `stateString`'s default arm — silent failure-look-
/// like under what the dispatcher considers a clean dispatch.
pub(crate) fn wrap_json_response<T: serde::Serialize>(
    response: &T,
) -> Result<Response<InvokeResponse>, Status> {
    let bytes = encode_json_payload(response)?;
    let invoke_response = InvokeResponse {
        result: bytes,
        result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
        state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
        ..InvokeResponse::default()
    };
    Ok(Response::new(invoke_response))
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
    fn targeted_envelope_has_full_tuple_and_nonce() {
        let hub = crate::core::ura::hub_ura("acme");
        let subject = crate::core::ura::owner_ability_ura(&hub, "federation.resolve")
            .expect("hub ability subject");
        let env = ProtoEnvelope::targeted("easynet:///r/acme/device/dev-a", &hub, &subject)
            .unwrap()
            .into_inner();
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
    fn targeted_rejects_provisional_caller() {
        let provisional = format!("provisional:{}", "a".repeat(64));
        let hub = crate::core::ura::hub_ura("acme");
        let subject = "easynet:///r/acme/device/dev-a";
        let err = ProtoEnvelope::targeted(provisional, hub, subject).unwrap_err();
        assert!(format!("{err}").contains("caller_ura is not a valid URA"));
    }

    #[test]
    fn federation_join_genesis_accepts_only_provisional_join_tuple() {
        let provisional = format!("provisional:{}", "a".repeat(64));
        let hub = crate::core::ura::hub_ura("acme");
        let membership = "easynet:///r/acme/device/dev-a";
        let env = ProtoEnvelope::federation_join_genesis(&provisional, &hub, membership)
            .unwrap()
            .into_inner();
        assert_eq!(env.caller.unwrap().ura, provisional);
        assert_eq!(env.callee.unwrap().ura, hub);
        assert_eq!(env.subject.unwrap().ura, membership);

        let cross_realm = ProtoEnvelope::federation_join_genesis(
            format!("provisional:{}", "b".repeat(64)),
            crate::core::ura::hub_ura("acme"),
            "easynet:///r/other/device/dev-a",
        )
        .unwrap_err();
        assert!(format!("{cross_realm}").contains("does not match hub realm"));
    }

    #[test]
    fn loopback_sets_caller_callee_subject_to_same_ura() {
        let req = ProtoEnvelope::loopback("easynet:///r/acme/device/dev-a")
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
        assert_eq!(req.function_name, "federation.discover");
    }

    #[test]
    fn hub_and_user_subject_refs_are_rejected() {
        let hub = crate::core::ura::hub_ura("acme");
        let hub_err =
            ProtoEnvelope::targeted("easynet:///r/acme/device/dev-a", &hub, &hub).unwrap_err();
        assert!(format!("{hub_err}").contains("subject_ref_kind_unsupported:Hub"));

        let user_err = ProtoEnvelope::targeted(
            "easynet:///r/acme/device/dev-a",
            &hub,
            "easynet:///r/acme/user/alice",
        )
        .unwrap_err();
        assert!(format!("{user_err}").contains("subject_ref_kind_unsupported:User"));
    }

    #[test]
    fn resource_subject_with_agent_path_segment_stays_resource() {
        let env = ProtoEnvelope::targeted(
            "easynet:///r/acme/device/dev-a",
            "easynet:///r/acme/device/dev-a",
            "easynet:///r/acme/resource/project/agent/audit-log",
        )
        .unwrap()
        .into_inner();
        let subject_ref = try_entity_ref(env.subject.unwrap().ura).unwrap();
        assert_eq!(subject_ref.kind, EntityRefKind::Resource as i32);
    }

    #[test]
    fn caller_only_keeps_tuple_incomplete_for_genesis_preludes() {
        let env = ProtoEnvelope::caller_only("easynet:///r/acme/device/dev-a")
            .unwrap()
            .into_inner();
        assert!(env.caller.is_some());
        assert!(env.callee.is_none());
        assert!(env.subject.is_none());
    }

    #[test]
    fn invalid_ura_is_rejected_before_wire_send() {
        let err = ProtoEnvelope::loopback("agent://self").unwrap_err();
        assert!(format!("{err}").contains("valid URA"));
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
        let envelope = ProtoEnvelope::targeted(
            "easynet:///r/acme/device/dev-a",
            callee,
            "easynet:///r/acme/device/dev-a",
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
            request
                .metadata
                .get(SIGNED_DESCRIPTOR_REF_METADATA_KEY)
                .map(String::as_str),
            Some(descriptor_ref.as_str())
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
                &descriptor.canonical_bytes(),
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
        let envelope = ProtoEnvelope::targeted(
            "easynet:///r/acme/device/dev-a",
            callee,
            "easynet:///r/acme/device/dev-a",
        )
        .unwrap();
        let descriptor = envelope
            .descriptor_bound_envelope(&descriptor_ref, &payload)
            .unwrap();
        let request = envelope
            .signed_descriptor_ref_invoke_request("echo", descriptor_ref, payload, &signer)
            .unwrap();

        assert_eq!(request.function_name, "echo");
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
                &descriptor.canonical_bytes(),
                &Signature::from_bytes(&signature_bytes),
            )
            .expect("signature must verify against explicit descriptor ref");
    }
}
