// EasyNet CLI — sidecar JSON frame model
// ======================================
//
// File: src/daemon/plugins/sidecar/frame.rs
// Description: Protocol values exchanged across the sidecar process boundary.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Daemon-owned invocation envelope sent to a sidecar process.
///
/// This is deliberately larger than `ability + args`: sidecars must receive the
/// same invocation identity the daemon uses for admission, receipts, and
/// causal ordering. What this is NOT: authority for sidecars to modify caller,
/// callee, subject, nonce, or causal context; the daemon constructs this value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SidecarInvocationEnvelope {
    /// Agent identity that initiated and signs the call.
    pub caller: String,
    /// Agent identity that exposes the selected ability.
    pub callee: String,
    /// Public callable contract selected on `callee`.
    pub ability: String,
    /// URA of the entity being acted on.
    pub subject: String,
    /// Caller-provided freshness material. The current daemon admission path
    /// expects 16 bytes, but this wire model stores bytes rather than a display
    /// string so no sidecar invents its own nonce encoding.
    pub invocation_nonce: Vec<u8>,
    /// Caller-declared causal placement. Canonical interpretation remains in
    /// Axon admission/receipt code; the sidecar receives the value for context.
    #[serde(default)]
    pub causal_context: Value,
    /// Ability-specific schema-conformant payload.
    #[serde(default)]
    pub args: Value,
}

/// Host-to-sidecar request frame.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SidecarRequestFrame {
    Invoke {
        call_id: String,
        invocation: SidecarInvocationEnvelope,
    },
    StreamOpen {
        call_id: String,
        invocation: SidecarInvocationEnvelope,
    },
    BidiOpen {
        call_id: String,
        invocation: SidecarInvocationEnvelope,
    },
    BidiInput {
        call_id: String,
        frame: Value,
    },
    Close {
        call_id: String,
        reason: String,
    },
}

/// Sidecar-to-host response frame.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SidecarResponseFrame {
    Result { call_id: String, value: Value },
    StreamItem { call_id: String, value: Value },
    BidiOutput { call_id: String, frame: Value },
    Terminal { call_id: String, reason: String },
    Error { call_id: String, message: String },
}
