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
/// ability invoke" contract.** CLI surfaces MUST go through this
/// function (or [`invoke_local_ability_with_subject`]), not the
/// transport-level free fns in `support::local_daemon_grpc`. The
/// indirection looks redundant — the body is one line — but it
/// matters: the day the local-ability transport evolves, this is
/// the **one** call site that knows the underlying transport.
/// Callers that bypass it become per-surface transport coupling.
pub fn invoke_local_ability(ability: &str, args: Value) -> anyhow::Result<Value> {
    invoke_local_ability_with_subject(ability, args, None)
}

/// Same as [`invoke_local_ability`] but threads an optional
/// envelope subject through to the daemon. The subject lands in
/// `EnvelopeContext.subject` for handlers that consume it
/// (e.g. `camera.snapshot`, which routes its frame from the
/// resource the subject URA names).
pub fn invoke_local_ability_with_subject(
    ability: &str,
    args: Value,
    subject: Option<String>,
) -> anyhow::Result<Value> {
    crate::support::platform::local_daemon_grpc::invoke_local_daemon_ability_with_subject(
        ability, args, subject,
    )
}

pub fn invoke_local_ability_with_subject_timeout(
    ability: &str,
    args: Value,
    subject: Option<String>,
    timeout: std::time::Duration,
) -> anyhow::Result<Value> {
    crate::support::platform::local_daemon_grpc::invoke_local_daemon_ability_with_subject_timeout(
        ability, args, subject, timeout,
    )
}

/// Invoke a canonical local Ability URA target through the daemon.
///
/// This path preserves the full descriptor owner identity in the signed
/// envelope. Use it for user-facing `ability invoke <ability-ura>` surfaces;
/// use the string-only helper only for daemon-owned system surfaces whose
/// callee really is the local device.
pub fn invoke_local_ability_target_with_subject_timeout(
    target: &LocalAbilityTarget,
    args: Value,
    subject: Option<String>,
    timeout: std::time::Duration,
) -> anyhow::Result<Value> {
    crate::support::platform::local_daemon_grpc::invoke_local_daemon_ability_targeted_timeout(
        target.dispatch_name(),
        args,
        target.callee_ura(),
        target.default_subject_ura(),
        subject,
        timeout,
    )
}

/// Stream a canonical local Ability URA target through the daemon.
///
/// This is the stream-mode twin of
/// [`invoke_local_ability_target_with_subject_timeout`]; it keeps callee and
/// default subject tied to the canonical Ability owner instead of defaulting
/// them to the local device signer.
pub fn invoke_local_ability_target_stream_with_subject(
    target: &LocalAbilityTarget,
    args: Value,
    subject: Option<String>,
    timeout: std::time::Duration,
    max_frames: Option<usize>,
) -> anyhow::Result<Vec<LocalStreamFrame>> {
    crate::support::platform::local_daemon_grpc::invoke_local_daemon_ability_targeted_stream_with_subject(
        target.dispatch_name(),
        args,
        target.callee_ura(),
        target.default_subject_ura(),
        subject,
        timeout,
        max_frames,
    )
}

/// Open a canonical local Ability URA target as an InvokeBidi JSON-frame
/// session and drain a bounded number of down frames.
pub fn invoke_local_ability_target_bidi_json_frames_with_subject(
    target: &LocalAbilityTarget,
    args: Value,
    subject: Option<String>,
    timeout: std::time::Duration,
    input_frames: Vec<Value>,
    max_frames: Option<usize>,
) -> anyhow::Result<Vec<LocalBidiFrame>> {
    crate::support::platform::local_daemon_grpc::invoke_local_daemon_ability_targeted_bidi_json_frames_with_subject(
        crate::support::platform::local_daemon_grpc::LocalDaemonTargetedBidiRequest {
            function_name: target.dispatch_name(),
            payload_json: args,
            callee_ura: target.callee_ura(),
            default_subject_ura: target.default_subject_ura(),
            subject,
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
    pub fn new(
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
    fn local_ability_target_preserves_agent_owner_as_callee_and_subject() {
        let selector = crate::core::ura::AbilitySelector::parse(
            "easynet:///r/acme/ability/alice.claude.weather",
        )
        .expect("agent ability selector");
        let target = LocalAbilityTarget::from_selector(&selector);

        assert_eq!(target.dispatch_name(), "claude.weather");
        assert_eq!(target.callee_ura(), "easynet:///r/acme/agent/alice.claude");
        assert_eq!(
            target.default_subject_ura(),
            "easynet:///r/acme/agent/alice.claude"
        );
    }

    #[test]
    fn local_ability_target_uses_ability_subject_for_hub_owner() {
        let selector = crate::core::ura::AbilitySelector::parse(
            "easynet:///r/acme/ability/hub.federation.resolve",
        )
        .expect("hub ability selector");
        let target = LocalAbilityTarget::from_selector(&selector);

        assert_eq!(target.dispatch_name(), "federation.resolve");
        assert_eq!(target.callee_ura(), "easynet:///r/acme/hub");
        assert_eq!(
            target.default_subject_ura(),
            "easynet:///r/acme/ability/hub.federation.resolve"
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
}
