// EasyNet CLI — Local ability invocation helper
// ==============================================
//
// File: src/support/local_invoke.rs
// Description: One function — `invoke_local_ability(name, args)` —
//              that every CLI subcommand uses to dispatch through
//              the local daemon's Axon Invocation gRPC surface
//              (~/.easynet/daemon.sock).
//
// Why this exists
// ---------------
// AXON-RFC-001 collapses every former "command" surface to one
// primitive: `Invoke <ability>`. Following that ontology in the
// CLI means each subcommand should be a thin wrapper that:
//
//   1. Maps the user's CLI args into a JSON args object.
//   2. Calls the appropriate ability via this helper.
//   3. Prints the result.
//
// Any subcommand that bypasses this — calling a transport
// directly, or constructing its own IPC client — is a layering
// violation: it ties the CLI to a specific transport (the
// federation bridge in pre-P1.5 code; an alternate IPC in some
// future variant) instead of to the ability surface. One helper
// here means one point to swap when the transport evolves.
//
// Routing model
// -------------
// Always local: the CLI is a thin client to the local daemon. A
// command that semantically needs a remote node (e.g. "show this
// device's siblings on the federation") must reach those nodes by
// invoking a federation-tier ability *on the local daemon*; the
// daemon is the only entity that holds federation transport
// state. The local-IPC contract here never grows a `--node` knob —
// federation routing belongs inside the ability, not in the CLI's
// dispatch path.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde_json::Value;

pub use crate::daemon::invocation::routing::target::LocalAbilityTarget;

/// One decoded frame from a daemon-hosted server-stream ability.
///
/// This is the CLI/support-layer projection of Axon's
/// `InvokeStreamChunk`: transport metadata stays visible, while the
/// business payload is decoded to JSON for frontend and script
/// consumers. It is not a live subscription handle; callers receive
/// a finite vector only after the helper has drained until terminal
/// or an explicit frame limit.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalStreamFrame {
    /// Zero-based frame sequence assigned by the daemon transport.
    pub sequence: u64,
    /// Content type advertised by the daemon for this frame.
    pub content_type: String,
    /// Whether this frame is terminal for the stream.
    pub terminal: bool,
    /// Decoded JSON business payload. Empty payloads decode to null.
    pub payload: Value,
}

/// One decoded down-frame from a daemon-hosted bidirectional ability.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalBidiFrame {
    /// Sequence assigned by the daemon transport.
    pub sequence: u64,
    /// Best-effort content type for the projected payload.
    pub content_type: String,
    /// Whether this frame terminates the bidi session.
    pub terminal: bool,
    /// JSON projection of the frame payload.
    pub payload: Value,
}

/// Project one Axon `InvokeBidiDown` frame into the support-layer JSON frame
/// shape consumed by CLI/product callers.
///
/// Binary chunks are payload bytes by definition, so non-JSON data is exposed
/// losslessly as `data_b64`. Receipt payloads are different: they are receipt
/// projection facts. A non-empty receipt payload must declare a JSON content
/// type and parse as JSON, otherwise the projection fails before product code
/// can mistake opaque bytes for verified receipt facts.
#[cfg(feature = "axon-pb")]
pub fn project_invoke_bidi_down_frame(
    frame: axon_sdk::pb::axon::v1::InvokeBidiDown,
) -> anyhow::Result<Option<LocalBidiFrame>> {
    use axon_sdk::pb::axon::v1::invoke_bidi_down::Payload as DownPayload;
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    use serde_json::json;

    let sequence = frame.sequence;
    let Some(payload) = frame.payload else {
        return Ok(None);
    };

    let projected = match payload {
        DownPayload::BinaryChunk(chunk) => {
            let payload = serde_json::from_slice(&chunk.data).unwrap_or_else(|_| {
                json!({
                    "type": "binary",
                    "stream_id": chunk.stream_id,
                    "data_b64": B64.encode(&chunk.data),
                })
            });
            LocalBidiFrame {
                sequence,
                content_type: "application/json".to_string(),
                terminal: false,
                payload,
            }
        }
        DownPayload::Receipt(receipt) => {
            let terminal =
                receipt.state != axon_sdk::invocation::InvocationState::Admitted.to_wire_i32();
            let receipt_payload =
                project_receipt_payload_json(&receipt.payload_content_type, &receipt.payload)?;
            LocalBidiFrame {
                sequence,
                content_type: receipt.payload_content_type.clone(),
                terminal,
                payload: json!({
                    "type": "receipt",
                    "state": receipt.state,
                    "reason": receipt.reason,
                    "cleanup_complete": receipt.cleanup_complete,
                    "failure": receipt.failure.map(|failure| json!({
                        "code": failure.code,
                        "message": failure.message,
                        "retryable": failure.retryable,
                    })),
                    "payload": receipt_payload,
                }),
            }
        }
        DownPayload::Control(_) => LocalBidiFrame {
            sequence,
            content_type: "application/json".to_string(),
            terminal: false,
            payload: json!({"type": "control"}),
        },
        DownPayload::DispatchCall(_) | DownPayload::ReverseDispatchResult(_) => return Ok(None),
    };

    Ok(Some(projected))
}

#[cfg(feature = "axon-pb")]
fn project_receipt_payload_json(content_type: &str, payload: &[u8]) -> anyhow::Result<Value> {
    if payload.is_empty() {
        return Ok(Value::Null);
    }
    if !is_json_content_type(content_type) {
        let content_type = if content_type.trim().is_empty() {
            "<missing>"
        } else {
            content_type.trim()
        };
        anyhow::bail!("InvokeBidi receipt payload declares non-JSON content_type `{content_type}`");
    }
    serde_json::from_slice(payload)
        .map_err(|err| anyhow::anyhow!("InvokeBidi receipt payload is not valid JSON: {err}"))
}

#[cfg(feature = "axon-pb")]
fn is_json_content_type(content_type: &str) -> bool {
    let essence = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    essence == "application/json" || essence.ends_with("+json")
}

/// Typed failure classes for local-daemon invocation (F-023).
///
/// Minted at the transport layer where the cause is structurally known
/// (socket probe failed, crate built without `axon-pb`), so consumers
/// branch with [`classify_invoke_error`] instead of sniffing message
/// text.
#[derive(Debug, thiserror::Error)]
pub enum LocalInvokeFailure {
    /// The daemon is not reachable (listener probe failed, or this
    /// build has no gRPC transport). Falling back to an in-process
    /// executor is legitimate — nothing ran.
    #[error("{0}")]
    DaemonOffline(String),
    /// The daemon accepted the transport connection and rejected or failed the
    /// invocation with a protocol status. The code remains structured so
    /// callers never infer control flow from daemon message wording.
    #[error("daemon error invoking {ability} through Axon (code={code}): {message}")]
    DaemonStatus {
        ability: String,
        code: LocalInvokeStatusCode,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalInvokeStatusCode {
    Ok,
    Cancelled,
    Unknown,
    InvalidArgument,
    DeadlineExceeded,
    NotFound,
    AlreadyExists,
    PermissionDenied,
    ResourceExhausted,
    FailedPrecondition,
    Aborted,
    OutOfRange,
    Unimplemented,
    Internal,
    Unavailable,
    DataLoss,
    Unauthenticated,
}

impl std::fmt::Display for LocalInvokeStatusCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Ok => "Ok",
            Self::Cancelled => "Cancelled",
            Self::Unknown => "Unknown",
            Self::InvalidArgument => "InvalidArgument",
            Self::DeadlineExceeded => "DeadlineExceeded",
            Self::NotFound => "NotFound",
            Self::AlreadyExists => "AlreadyExists",
            Self::PermissionDenied => "PermissionDenied",
            Self::ResourceExhausted => "ResourceExhausted",
            Self::FailedPrecondition => "FailedPrecondition",
            Self::Aborted => "Aborted",
            Self::OutOfRange => "OutOfRange",
            Self::Unimplemented => "Unimplemented",
            Self::Internal => "Internal",
            Self::Unavailable => "Unavailable",
            Self::DataLoss => "DataLoss",
            Self::Unauthenticated => "Unauthenticated",
        })
    }
}

#[cfg(feature = "axon-pb")]
impl From<tonic::Code> for LocalInvokeStatusCode {
    fn from(code: tonic::Code) -> Self {
        match code {
            tonic::Code::Ok => Self::Ok,
            tonic::Code::Cancelled => Self::Cancelled,
            tonic::Code::Unknown => Self::Unknown,
            tonic::Code::InvalidArgument => Self::InvalidArgument,
            tonic::Code::DeadlineExceeded => Self::DeadlineExceeded,
            tonic::Code::NotFound => Self::NotFound,
            tonic::Code::AlreadyExists => Self::AlreadyExists,
            tonic::Code::PermissionDenied => Self::PermissionDenied,
            tonic::Code::ResourceExhausted => Self::ResourceExhausted,
            tonic::Code::FailedPrecondition => Self::FailedPrecondition,
            tonic::Code::Aborted => Self::Aborted,
            tonic::Code::OutOfRange => Self::OutOfRange,
            tonic::Code::Unimplemented => Self::Unimplemented,
            tonic::Code::Internal => Self::Internal,
            tonic::Code::Unavailable => Self::Unavailable,
            tonic::Code::DataLoss => Self::DataLoss,
            tonic::Code::Unauthenticated => Self::Unauthenticated,
        }
    }
}

/// Consumer-facing classification of a local-invoke error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalInvokeErrorKind {
    DaemonOffline,
    AbilityUnregistered,
    /// The daemon executed the request and it failed for real.
    /// Re-running through another executor would double-execute a
    /// side-effecting ability to mask a true error.
    Failed,
}

/// Classify a local-invoke error for fallback decisions.
///
/// Walks the anyhow chain for the transport-owned typed failure. Untyped
/// errors are real execution/projection failures and therefore never grant a
/// fallback executor permission to run the request again.
pub fn classify_invoke_error(err: &anyhow::Error) -> LocalInvokeErrorKind {
    for cause in err.chain() {
        if let Some(f) = cause.downcast_ref::<LocalInvokeFailure>() {
            return match f {
                LocalInvokeFailure::DaemonOffline(_) => LocalInvokeErrorKind::DaemonOffline,
                LocalInvokeFailure::DaemonStatus {
                    code: LocalInvokeStatusCode::NotFound,
                    ..
                } => LocalInvokeErrorKind::AbilityUnregistered,
                LocalInvokeFailure::DaemonStatus { .. } => LocalInvokeErrorKind::Failed,
            };
        }
    }
    LocalInvokeErrorKind::Failed
}

/// Invoke an ability against the local daemon's Axon runtime.
///
/// `ability` is the wire-level qualified name (e.g. `easynet.discover`,
/// `claude.weather`, `observe.health`). `args` is forwarded as-is —
/// the helper does not validate the shape; the daemon-side handler
/// is the authority on argument validation, and a CLI-side
/// pre-check would only drift.
///
/// On success returns the raw value (whatever shape the handler
/// produced). On error returns a typed `anyhow::Error` with the
/// daemon-side `code` + `message` rendered into the message — the
/// CLI's outer layer can surface that verbatim or pattern-match if
/// it needs typed handling.
///
/// **Canonical entry point for the "one CLI subcommand = one
/// ability invoke" contract.** CLI surfaces that invoke daemon-self
/// root abilities use this function. Surfaces that intentionally bind
/// a daemon-system root invocation to an explicit subject use
/// [`LocalDaemonSystemAbilityIssuer`]. The indirection looks redundant
/// — the body is one line — but it matters: the day the local-ability
/// transport evolves, this is the **one** call site that knows the
/// underlying transport. Callers that bypass it become per-surface
/// transport coupling.
pub fn invoke_local_ability(ability: &str, args: Value) -> anyhow::Result<Value> {
    crate::support::platform::local_daemon_grpc::invoke_local_daemon_ability(ability, args)
}

/// Named issuer for product CLI commands that invoke daemon-local abilities as
/// `_system.local` roots while preserving the ability owner's callee identity.
///
/// Generic `easynet ability ...` ingress must use explicit tuple helpers. This
/// issuer exists for product commands such as pages, principal, and media
/// record workflows whose user-facing contract is not raw invocation tuple
/// submission.
pub struct LocalDaemonSystemAbilityIssuer;

impl LocalDaemonSystemAbilityIssuer {
    pub fn invoke_root_for_subject(
        ability: &str,
        args: Value,
        subject_ura: &str,
    ) -> anyhow::Result<Value> {
        Self::invoke_root_for_subject_timeout(
            ability,
            args,
            subject_ura,
            std::time::Duration::from_secs(30),
        )
    }

    pub fn invoke_root_for_subject_timeout(
        ability: &str,
        args: Value,
        subject_ura: &str,
        timeout: std::time::Duration,
    ) -> anyhow::Result<Value> {
        crate::support::platform::local_daemon_grpc::invoke_local_daemon_system_ability_root_for_subject_timeout(
            ability,
            args,
            subject_ura,
            timeout,
        )
    }

    pub fn invoke_target_root_timeout(
        target: &LocalAbilityTarget,
        args: Value,
        subject_ura: &str,
        timeout: std::time::Duration,
    ) -> anyhow::Result<Value> {
        crate::support::platform::local_daemon_grpc::invoke_local_daemon_system_ability_targeted_root_timeout(
            target.dispatch_name(),
            args,
            target.callee_ura(),
            subject_ura,
            timeout,
        )
    }

    pub fn invoke_target_root_derived_subject_timeout(
        target: &LocalAbilityTarget,
        args: Value,
        timeout: std::time::Duration,
    ) -> anyhow::Result<Value> {
        let subject_ura = target.daemon_system_subject_ura()?;
        Self::invoke_target_root_timeout(target, args, &subject_ura, timeout)
    }

    pub fn stream_target_root(
        target: &LocalAbilityTarget,
        args: Value,
        subject_ura: &str,
        timeout: std::time::Duration,
        max_frames: Option<usize>,
    ) -> anyhow::Result<Vec<LocalStreamFrame>> {
        crate::support::platform::local_daemon_grpc::invoke_local_daemon_system_ability_targeted_stream_root(
            target.dispatch_name(),
            args,
            target.callee_ura(),
            subject_ura,
            timeout,
            max_frames,
        )
    }
}

/// Named issuer for daemon-local runtime-state reads.
///
/// These reads are product/operator projections over the running LocalRuntime
/// state: ability catalogue, health/status probes, and invocation ledger views.
/// They must not enter transport through the generic daemon-self subject
/// shortcut because that hides the semantic subject until admission fails. The
/// issuer binds every read to the daemon identity published by control discovery
/// before crossing the local Axon gRPC boundary.
pub struct LocalRuntimeStateReadIssuer;

impl LocalRuntimeStateReadIssuer {
    pub fn invoke(ability: &str, args: Value) -> anyhow::Result<Value> {
        Self::invoke_timeout(ability, args, std::time::Duration::from_secs(30))
    }

    pub fn invoke_timeout(
        ability: &str,
        args: Value,
        timeout: std::time::Duration,
    ) -> anyhow::Result<Value> {
        let subject_ura = Self::subject_ura()?;
        LocalDaemonSystemAbilityIssuer::invoke_root_for_subject_timeout(
            ability,
            args,
            &subject_ura,
            timeout,
        )
    }

    fn subject_ura() -> anyhow::Result<String> {
        crate::daemon::identity::local_invocation::local_daemon_ura()
            .map_err(|error| anyhow::anyhow!("runtime-state read subject unavailable: {error}"))
    }
}

/// Invoke a canonical local target with public-ingress tuple facts.
///
/// This is the user-facing ability-invoke path: subject, nonce, and root
/// causal placement are declared by the caller before daemon transport entry.
pub fn invoke_local_ability_target_explicit_root_timeout(
    target: &LocalAbilityTarget,
    args: Value,
    subject_ura: &str,
    invocation_nonce: [u8; 16],
    timeout: std::time::Duration,
) -> anyhow::Result<Value> {
    crate::support::platform::local_daemon_grpc::invoke_local_daemon_ability_targeted_explicit_root_timeout(
        target.dispatch_name(),
        args,
        target.callee_ura(),
        subject_ura,
        invocation_nonce,
        timeout,
    )
}

/// Stream a canonical local Ability URA target with public-ingress tuple facts.
pub fn invoke_local_ability_target_stream_explicit_root(
    target: &LocalAbilityTarget,
    args: Value,
    subject_ura: &str,
    invocation_nonce: [u8; 16],
    timeout: std::time::Duration,
    max_frames: Option<usize>,
) -> anyhow::Result<Vec<LocalStreamFrame>> {
    crate::support::platform::local_daemon_grpc::invoke_local_daemon_ability_targeted_stream_explicit_root(
        target.dispatch_name(),
        args,
        target.callee_ura(),
        subject_ura,
        invocation_nonce,
        timeout,
        max_frames,
    )
}

/// Open a canonical local Ability URA target as an InvokeBidi JSON-frame
/// session and drain a bounded number of down frames.
pub fn invoke_local_ability_target_bidi_json_frames_explicit_root(
    target: &LocalAbilityTarget,
    args: Value,
    subject_ura: &str,
    invocation_nonce: [u8; 16],
    timeout: std::time::Duration,
    input_frames: Vec<Value>,
    max_frames: Option<usize>,
) -> anyhow::Result<Vec<LocalBidiFrame>> {
    crate::support::platform::local_daemon_grpc::invoke_local_daemon_ability_targeted_bidi_json_frames_explicit_root(
        crate::support::platform::local_daemon_grpc::LocalDaemonTargetedBidiRequest {
            function_name: target.dispatch_name(),
            payload_json: args,
            callee_ura: target.callee_ura(),
            subject_ura,
            invocation_nonce,
            timeout,
            input_frames,
            max_frames,
        },
    )
}

/// Metadata produced only after Axon has verified the admission and terminal
/// receipt checkpoints against the trusted local key service.
///
/// The constructor remains private to this module. Consumers may inspect or
/// serialize the verified JSON projection, but cannot label arbitrary JSON as
/// verified metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct VerifiedLocalInvocationMeta(Value);

impl VerifiedLocalInvocationMeta {
    pub fn as_value(&self) -> &Value {
        &self.0
    }
}

/// Explicit caller-owned facts for a daemon-local system invocation.
///
/// The caller identity is fixed by the named `_system.local` issuer. Callee and
/// ability come from [`LocalAbilityTarget`]. Subject, nonce, causal placement,
/// timeout, and trace placement must be selected before transport entry.
#[derive(Debug)]
pub struct LocalSystemInvocationContext<'a> {
    subject_ura: String,
    invocation_nonce: [u8; 16],
    causal_parents: &'a [Value],
    step_timeout: std::time::Duration,
    trace_id: Option<&'a str>,
}

impl<'a> LocalSystemInvocationContext<'a> {
    fn new(
        subject_ura: impl Into<String>,
        invocation_nonce: [u8; 16],
        causal_parents: &'a [Value],
        step_timeout: std::time::Duration,
        trace_id: Option<&'a str>,
    ) -> anyhow::Result<Self> {
        let subject_ura = subject_ura.into();
        let subject_ura = subject_ura.trim();
        if subject_ura.is_empty() {
            anyhow::bail!("local system invocation subject_ura must not be empty");
        }
        crate::core::ura::parse_ura(subject_ura).map_err(|error| {
            anyhow::anyhow!("local system invocation subject_ura is invalid: {error}")
        })?;
        if invocation_nonce == [0; 16] {
            anyhow::bail!("local system invocation nonce must not be all-zero");
        }
        if step_timeout.is_zero() {
            anyhow::bail!("local system invocation timeout must be greater than zero");
        }
        if trace_id.is_some_and(|trace_id| trace_id.trim().is_empty()) {
            anyhow::bail!("local system invocation trace_id must not be empty when supplied");
        }
        Ok(Self {
            subject_ura: subject_ura.to_string(),
            invocation_nonce,
            causal_parents,
            step_timeout,
            trace_id,
        })
    }
}

/// Named issuer for daemon-local system contexts used by product adapters.
///
/// Callers provide the semantic subject, causal parents, timeout, and trace.
/// Freshness is minted only here so adapters do not own root tuple facts.
pub struct LocalSystemInvocationIssuer;

impl LocalSystemInvocationIssuer {
    pub fn root_context<'a>(
        subject_ura: impl Into<String>,
        causal_parents: &'a [Value],
        step_timeout: std::time::Duration,
        trace_id: Option<&'a str>,
    ) -> anyhow::Result<LocalSystemInvocationContext<'a>> {
        LocalSystemInvocationContext::new(
            subject_ura,
            axon_sdk::invocation::fresh_nonce(),
            causal_parents,
            step_timeout,
            trace_id,
        )
    }

    pub fn root_context_for_target<'a>(
        target: &LocalAbilityTarget,
        causal_parents: &'a [Value],
        step_timeout: std::time::Duration,
        trace_id: Option<&'a str>,
    ) -> anyhow::Result<LocalSystemInvocationContext<'a>> {
        Self::root_context(
            target.daemon_system_subject_ura()?,
            causal_parents,
            step_timeout,
            trace_id,
        )
    }
}

/// Invoke a canonical local Ability target and return cryptographically
/// verified invocation metadata with the result.
pub fn invoke_local_ability_target_with_invocation_meta(
    target: &LocalAbilityTarget,
    args: Value,
    context: LocalSystemInvocationContext<'_>,
) -> anyhow::Result<(Value, VerifiedLocalInvocationMeta)> {
    let request =
        crate::support::platform::local_daemon_grpc::LocalDaemonTargetedInvocationMetaRequest {
            function_name: target.dispatch_name(),
            payload_json: args,
            callee_ura: target.callee_ura(),
            subject_ura: &context.subject_ura,
            invocation_nonce: context.invocation_nonce,
            causal_parents: context.causal_parents,
            step_timeout: context.step_timeout,
            trace_id: context.trace_id,
        };
    let (value, metadata) =
        crate::support::platform::local_daemon_grpc::invoke_local_daemon_ability_targeted_with_invocation_meta(request)?;
    Ok((value, VerifiedLocalInvocationMeta(metadata)))
}

/// Invoke a canonical local target with explicit hosted-agent delegation.
///
/// This does NOT rewrite the hosted agent into Axon's caller. The signed
/// Invocation caller is the local daemon IPC system identity; hosted-agent
/// intent is carried as explicit delegation metadata and ability arguments,
/// not by rewriting caller identity.
pub fn invoke_local_ability_target_with_hosted_agent_delegation(
    target: &LocalAbilityTarget,
    args: Value,
    context: LocalSystemInvocationContext<'_>,
    hosted_agent_ura: &str,
) -> anyhow::Result<(Value, Value)> {
    let request =
        crate::support::platform::local_daemon_grpc::LocalDaemonTargetedInvocationMetaRequest {
            function_name: target.dispatch_name(),
            payload_json: args,
            callee_ura: target.callee_ura(),
            subject_ura: &context.subject_ura,
            invocation_nonce: context.invocation_nonce,
            causal_parents: context.causal_parents,
            step_timeout: context.step_timeout,
            trace_id: context.trace_id,
        };
    crate::support::platform::local_daemon_grpc::invoke_local_daemon_ability_targeted_with_hosted_agent_delegation(
        request,
        hosted_agent_ura,
    )
}

/// Standard error message for any CLI surface that semantically
/// requires the federation tier (cross-node enumeration, remote
/// dispatch, voice/video signaling). The federation Invoke surface
/// that would back these calls was removed by AXON-RFC-001 P1.5
/// and ships as a follow-up; until then, every command that
/// genuinely needs cross-node reach surfaces this exact message.
///
/// Centralised so:
///   * the wording stays byte-identical across surfaces (a script
///     can grep one substring),
///   * the operator sees one consistent name for the missing
///     subsystem instead of 8 variations of "federation gone",
///   * the day federation Invoke lands, deletion of this string
///     plus its callers is one PR rather than scavenger-hunt.
///
/// `action` is a short verb-phrase describing what the user was
/// trying to do (e.g. `"list remote devices"`, `"deploy ability to a
/// remote node"`); it is splice into the message so the operator
/// sees the verb that failed in front of the same explanation.
#[cfg(not(feature = "axon-pb"))]
pub fn federation_not_wired_error(action: &str) -> anyhow::Error {
    anyhow::Error::new(LocalInvokeFailure::DaemonOffline(format!(
        "{action} requires the `axon-pb` feature; rebuild with \
         `cargo build --features axon-pb` (production builds always do)."
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn local_system_context_requires_complete_explicit_facts() {
        let complete = LocalSystemInvocationContext::new(
            "easynet:///r/acme/resource/device.local/probe/alive",
            [0x33; 16],
            &[],
            std::time::Duration::from_secs(5),
            Some("trace-1"),
        )
        .expect("complete context");
        assert_eq!(complete.invocation_nonce, [0x33; 16]);
        assert_eq!(complete.step_timeout, std::time::Duration::from_secs(5));

        assert!(LocalSystemInvocationContext::new(
            "",
            [0x33; 16],
            &[],
            std::time::Duration::from_secs(5),
            None,
        )
        .is_err());
        assert!(LocalSystemInvocationContext::new(
            "easynet:///r/acme/resource/device.local/probe/alive",
            [0; 16],
            &[],
            std::time::Duration::from_secs(5),
            None,
        )
        .is_err());
        assert!(LocalSystemInvocationContext::new(
            "easynet:///r/acme/resource/device.local/probe/alive",
            [0x33; 16],
            &[],
            std::time::Duration::ZERO,
            None,
        )
        .is_err());
    }

    #[test]
    fn local_system_context_for_agent_target_uses_agent_owner_subject() {
        let selector = crate::core::ura::AbilitySelector::parse(
            "easynet:///r/acme/ability/alice.claude.weather",
        )
        .expect("agent ability selector");
        let target = LocalAbilityTarget::from_selector(&selector);
        let context = LocalSystemInvocationIssuer::root_context_for_target(
            &target,
            &[],
            std::time::Duration::from_secs(5),
            None,
        )
        .expect("issuer context");

        assert_eq!(target.dispatch_name(), "claude.weather");
        assert_eq!(target.callee_ura(), "easynet:///r/acme/agent/alice.claude");
        assert_eq!(context.subject_ura, "easynet:///r/acme/agent/alice.claude");
    }

    #[test]
    fn local_system_context_for_hub_target_uses_ability_subject() {
        let selector = crate::core::ura::AbilitySelector::parse(
            "easynet:///r/acme/ability/authority.federation.resolve",
        )
        .expect("hub ability selector");
        let target = LocalAbilityTarget::from_selector(&selector);
        let context = LocalSystemInvocationIssuer::root_context_for_target(
            &target,
            &[],
            std::time::Duration::from_secs(5),
            None,
        )
        .expect("issuer context");

        assert_eq!(target.dispatch_name(), "federation.resolve");
        assert_eq!(target.callee_ura(), "easynet:///r/acme/authority");
        assert_eq!(
            context.subject_ura,
            "easynet:///r/acme/ability/authority.federation.resolve"
        );
    }

    #[test]
    fn classification_uses_typed_daemon_status_not_message_text() {
        let not_found = anyhow::Error::new(LocalInvokeFailure::DaemonStatus {
            ability: "skill.list".to_string(),
            code: LocalInvokeStatusCode::NotFound,
            message: "wording may change".to_string(),
        });
        assert_eq!(
            classify_invoke_error(&not_found),
            LocalInvokeErrorKind::AbilityUnregistered
        );

        let untyped = anyhow::anyhow!("unknown_ability and daemon not running are only text");
        assert_eq!(
            classify_invoke_error(&untyped),
            LocalInvokeErrorKind::Failed
        );
    }

    #[test]
    fn non_not_found_daemon_status_cannot_authorize_fallback_execution() {
        let unavailable = anyhow::Error::new(LocalInvokeFailure::DaemonStatus {
            ability: "agent.start".to_string(),
            code: LocalInvokeStatusCode::Unavailable,
            message: "daemon reported a runtime failure".to_string(),
        });
        assert_eq!(
            classify_invoke_error(&unavailable),
            LocalInvokeErrorKind::Failed
        );
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn bidi_down_projection_preserves_binary_chunk_as_lossless_b64() {
        use axon_sdk::pb::axon::v1::{
            invoke_bidi_down::Payload as DownPayload, BinaryChunk, InvokeBidiDown,
        };

        let frame = project_invoke_bidi_down_frame(InvokeBidiDown {
            sequence: 7,
            payload: Some(DownPayload::BinaryChunk(BinaryChunk {
                stream_id: 3,
                data: vec![0xff, 0x00, 0x01],
                ..BinaryChunk::default()
            })),
            ..InvokeBidiDown::default()
        })
        .expect("binary chunk projection")
        .expect("binary chunk frame");

        assert_eq!(frame.sequence, 7);
        assert_eq!(frame.payload["type"], "binary");
        assert_eq!(frame.payload["stream_id"], 3);
        assert_eq!(frame.payload["data_b64"], "/wAB");
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn bidi_receipt_projection_rejects_non_json_payload_content_type() {
        use axon_sdk::pb::axon::v1::{
            invoke_bidi_down::Payload as DownPayload, InvocationReceipt, InvokeBidiDown,
        };

        let error = project_invoke_bidi_down_frame(InvokeBidiDown {
            payload: Some(DownPayload::Receipt(InvocationReceipt {
                payload_content_type: "application/octet-stream".to_string(),
                payload: vec![1, 2, 3],
                ..InvocationReceipt::default()
            })),
            ..InvokeBidiDown::default()
        })
        .expect_err("receipt payload content_type must be JSON");

        assert!(
            error.to_string().contains("non-JSON content_type"),
            "wrong error: {error}"
        );
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn bidi_receipt_projection_rejects_malformed_json_payload() {
        use axon_sdk::pb::axon::v1::{
            invoke_bidi_down::Payload as DownPayload, InvocationReceipt, InvokeBidiDown,
        };

        let error = project_invoke_bidi_down_frame(InvokeBidiDown {
            payload: Some(DownPayload::Receipt(InvocationReceipt {
                payload_content_type: "application/json".to_string(),
                payload: b"{not-json".to_vec(),
                ..InvocationReceipt::default()
            })),
            ..InvokeBidiDown::default()
        })
        .expect_err("receipt payload JSON must parse");

        assert!(
            error.to_string().contains("not valid JSON"),
            "wrong error: {error}"
        );
    }

    #[test]
    fn invoke_local_ability_surfaces_daemon_down_with_actionable_message() {
        // Fresh HOME: no Axon daemon socket can be accepting. The
        // canonical helper must surface the same actionable
        // daemon-down message while routing through daemon.sock,
        // not the legacy control socket frame.
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let err =
            invoke_local_ability("observe.health", json!({})).expect_err("daemon-down must fail");
        let msg = format!("{err}");
        assert!(
            msg.contains("daemon not running"),
            "must say `daemon not running`; got: {msg}"
        );
        assert!(
            msg.contains("easynet runtime start"),
            "must point at `easynet [runtime] start`; got: {msg}"
        );
    }

    #[test]
    fn runtime_state_read_subject_requires_control_discovery_identity() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let error = LocalRuntimeStateReadIssuer::subject_ura()
            .expect_err("runtime-state reads must not synthesize daemon identity");
        let message = format!("{error:#}");
        assert!(
            message.contains("runtime-state read subject unavailable"),
            "wrong readiness error: {message}"
        );
        assert!(
            message.contains("control discovery does not publish a daemon identity"),
            "runtime-state read must fail before default/device fallback: {message}"
        );
    }
}
