// EasyNet CLI — Axon invocation wire builders
// ===========================================
//
// File: src/services/invocation_transport/invocation_wire.rs
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
// This is the wire-facade counterpart to `crate::ura` (canonical URA
// construction/parsing) and `runtime::invocation` (domain invocation
// records). It does not perform admission. When a caller provides a
// `SelfIdentity`, it can attach the caller signature over Axon's
// descriptor-bound canonical bytes so production daemon IPC has one
// wire-shape construction point.

use rand::RngCore;

use anyhow::Context as _;
use tonic::{Response, Status};

use easynet_axon::invocation::DescriptorBoundEnvelope;
use easynet_axon::pb::axon::v1::{
    causal_context, AgentIdentity, CallerSignature, CausalContext, Empty, EntityRef, EntityRefKind,
    Envelope, InvokeRequest, InvokeResponse, SubjectIdentity,
};

use crate::runtime::axon_bridge::wire_descriptor::{
    descriptor_bound_from_wire_parts, WireCallerIdentity,
};

pub const DEFAULT_URA_PROFILE: &str = "easynet-strict-v2";

/// Invocation metadata keys carrying caller-authority proofs
/// (single wire-contract source; admission verifies, ledger records).
pub(crate) const DELEGATION_METADATA_KEY: &str = "x-easynet-delegation";
pub(crate) const SESSION_AUTHORITY_METADATA_KEY: &str = "x-easynet-session-authority";

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

    #[must_use]
    pub fn into_inner(self) -> Envelope {
        self.inner
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

    /// Sign this full descriptor-bound envelope and build an InvokeRequest.
    ///
    /// This is the local-daemon client path: the envelope remains ordinary
    /// external caller material, so Axon admission later verifies the
    /// `caller_signature` against the caller's trust anchor. Daemon-internal
    /// `_system.local` calls use `LocalRuntimeIngress::LocalSystem` instead.
    pub fn signed_invoke_request(
        self,
        function_name: impl Into<String>,
        arguments: Vec<u8>,
        signer: &dyn crate::services::self_identity::SelfIdentity,
    ) -> anyhow::Result<InvokeRequest> {
        let function_name = function_name.into();
        if function_name.trim().is_empty() {
            anyhow::bail!("function_name must not be empty");
        }
        let signed = self.sign_descriptor_bound(&function_name, &arguments, signer)?;
        Ok(InvokeRequest {
            envelope: Some(signed.into_inner()),
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
        signer: &dyn crate::services::self_identity::SelfIdentity,
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
        Ok(InvokeRequest {
            envelope: Some(signed.into_inner()),
            function_name,
            arguments,
            ..InvokeRequest::default()
        })
    }

    /// Attach an Ed25519 caller signature over Axon's
    /// `DescriptorBoundEnvelope::canonical_bytes()`.
    pub fn sign_descriptor_bound(
        mut self,
        ability: &str,
        arguments: &[u8],
        signer: &dyn crate::services::self_identity::SelfIdentity,
    ) -> anyhow::Result<Self> {
        let descriptor = self.descriptor_bound_envelope(ability, arguments)?;
        let caller_ura = descriptor.envelope().caller.ura.clone();
        let signature = signer
            .sign(&caller_ura, &descriptor.canonical_bytes())
            .with_context(|| format!("sign descriptor-bound invocation as {caller_ura}"))?;
        self.inner.caller_signature = Some(CallerSignature {
            algorithm: "ed25519".to_string(),
            signature: signature.to_bytes().to_vec(),
            key_id_hint: caller_ura,
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
    crate::ura::parse_ura(&ura).map_err(|e| anyhow::anyhow!("{field} is not a valid URA: {e}"))?;
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
    match crate::ura::parse_ura(ura.trim()).map(|parsed| parsed.kind) {
        Ok(crate::ura::URAKind::Agent) => Ok(EntityRefKind::Agent),
        Ok(crate::ura::URAKind::Ability) => Ok(EntityRefKind::Ability),
        Ok(crate::ura::URAKind::Device) => Ok(EntityRefKind::Device),
        Ok(crate::ura::URAKind::Resource) => Ok(EntityRefKind::Resource),
        Ok(other) => anyhow::bail!("subject_ref_kind_unsupported:{other:?}"),
        Err(err) => anyhow::bail!("subject_ref_ura_parse_failed:{err}"),
    }
}

fn top_level_subject_entity_kind(ura: &str) -> Option<EntityRefKind> {
    let rest = ura.trim().strip_prefix(crate::ura::URA_SCHEME)?;
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
    crate::ura::parse_ura(target_ura)
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
    let bytes = serde_json::to_vec(response).map_err(|err| {
        Status::internal(format!(
            "federation wrapper: failed to encode JSON response: {err}"
        ))
    })?;
    let invoke_response = InvokeResponse {
        result: bytes,
        result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
        state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
        ..InvokeResponse::default()
    };
    Ok(Response::new(invoke_response))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};

    struct TestSigner(SigningKey);

    impl crate::services::self_identity::SelfIdentity for TestSigner {
        fn sign(
            &self,
            _self_ura: &str,
            canonical_bytes: &[u8],
        ) -> Result<Signature, crate::services::self_identity::SelfIdentityError> {
            Ok(self.0.sign(canonical_bytes))
        }

        fn public_key(
            &self,
            _self_ura: &str,
        ) -> Result<VerifyingKey, crate::services::self_identity::SelfIdentityError> {
            Ok(self.0.verifying_key())
        }
    }

    #[test]
    fn targeted_envelope_has_full_tuple_and_nonce() {
        let hub = crate::ura::hub_ura("acme");
        let subject =
            crate::ura::owner_ability_ura(&hub, "federation.resolve").expect("hub ability subject");
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
        let hub = crate::ura::hub_ura("acme");
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
        let envelope = ProtoEnvelope::targeted(
            "easynet:///r/acme/device/dev-a",
            "easynet:///r/acme/device/dev-a",
            "easynet:///r/acme/device/dev-a",
        )
        .unwrap();
        let descriptor = envelope
            .descriptor_bound_envelope("demo.echo", &payload)
            .unwrap();
        let request = envelope
            .signed_invoke_request("demo.echo", payload, &signer)
            .unwrap();
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
            "{}@2.3.0",
            crate::ura::owner_ability_ura(callee, "echo").unwrap()
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
