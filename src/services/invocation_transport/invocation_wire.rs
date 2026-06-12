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
// records). It does not perform admission or signing.

use rand::RngCore;

use tonic::{Response, Status};

use easynet_axon::pb::axon::v1::{
    AgentIdentity, Envelope, InvokeRequest, InvokeResponse, SubjectIdentity,
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
        Ok(Self {
            inner: Envelope {
                caller: Some(agent_identity(caller_ura)),
                callee: Some(agent_identity(callee_ura)),
                subject: Some(subject_identity(subject_ura)),
                invocation_nonce: fresh_invocation_nonce().to_vec(),
                ..Envelope::default()
            },
        })
    }

    #[must_use]
    pub fn into_inner(self) -> Envelope {
        self.inner
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

fn fresh_invocation_nonce() -> [u8; 16] {
    let mut nonce = [0_u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    nonce
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

    #[test]
    fn targeted_envelope_has_full_tuple_and_nonce() {
        let hub = crate::ura::hub_ura("acme");
        let env = ProtoEnvelope::targeted(
            "easynet:///r/acme/device/dev-a",
            &hub,
            "easynet:///r/acme/user/alice",
        )
        .unwrap()
        .into_inner();
        assert_eq!(env.caller.unwrap().ura, "easynet:///r/acme/device/dev-a");
        assert_eq!(env.callee.unwrap().ura, hub);
        assert_eq!(env.subject.unwrap().ura, "easynet:///r/acme/user/alice");
        assert_eq!(env.invocation_nonce.len(), 16);
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
        assert_eq!(req.function_name, "federation.discover");
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
}
