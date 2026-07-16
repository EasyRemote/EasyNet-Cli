// EasyNet CLI — daemon runtime invocation record
// ==============================================
//
// File: src/daemon/invocation/receipts/runtime_record.rs
// Description: Daemon-local adapter record for scheduled/loop/kernel
//              execution, plus the Receipt terminal record used by
//              daemon-internal runtime services.
//
// Why this module exists
// ----------------------
// The public Invocation primitive belongs to Axon. This module does
// not define canonical Invocation semantics and does not sign or
// verify protocol bytes. It exists only because a few daemon-owned
// services still need an in-process record to carry caller/callee/
// ability/subject/nonce/args into `Kernel::invoke`.
//
// The runtime id for this adapter is derived from an explicit
// daemon-local session-key encoding. It deliberately is not Axon's
// descriptor-bound wire envelope: the session key must be computable
// before runtime descriptor negotiation and must remain stable across
// descriptor-version changes.
//
// Signature status
// ----------------
// `caller_signature` is optional only for daemon-local `_system.local`
// loopback calls. Kernel turns those into Axon public signed requests with
// the synthetic system key; every public transport caller, including Device,
// User, Agent, Hub, and Backend, must carry the caller signature and Axon
// remains the owner of verification, replay, and receipt proof semantics.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use easynet_axon::invocation::InvocationState;

/// Unified Resource Address.
///
/// URAs are canonical `easynet:///r/<realm>/...` addresses. New code
/// should construct them through `crate::core::ura` builders and validate
/// externally supplied values with `crate::core::ura::parse_ura`. Plain
/// node ids, agent ids, or ad-hoc route fragments are not valid
/// Runtime invocation URAs.
pub type Ura = String;

/// Ability member-call name.
///
/// This is the registry function name (`<agent>.<verb>`,
/// `skill.list`, `federation.resolve_key`, ...). The route or
/// resource URA that locates a published ability is carried by the
/// envelope/registry layers, not by this dispatch key.
pub type AbilityName = String;

/// Daemon-local causal-context adapter shapes.
///
/// - `Null`   — a freshly-initiated runtime invocation with no prior receipt
///              in its causal past (e.g. a user-initiated Client FFI
///              call that did not cite any prior receipt).
/// - `Scalar` — a single prior invocation id. This is not sufficient
///              for Axon canonical causal encoding.
/// - `List`   — multiple prior invocation ids forming a set causal
///              parent. This is also not sufficient for Axon canonical
///              causal encoding.
/// - `Merkle` — a Merkle root placeholder without an Axon proof
///              URA.
///
/// v1 emits `Null` and `Scalar` only (schedule tick / loop controller
/// / permission admission all have at most one prior receipt to cite).
/// `List` and `Merkle` variants exist on the wire so v2 causal
/// scheduling can populate them without a schema migration.
///
/// This is retained for daemon-internal records only. Axon's canonical
/// causal context requires receipt hashes and receipt URAs; the
/// `Scalar`/`List` variants here carry only prior invocation ids and
/// therefore cannot be encoded into Axon canonical bytes without
/// losing verification semantics. `runtime_invocation_id` rejects
/// those variants until callers supply Axon `ReceiptRef`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeCausalContext {
    Null,
    Scalar { prior_invocation_id: String },
    List { prior_invocation_ids: Vec<String> },
    Merkle { merkle_root: String },
}

impl RuntimeCausalContext {
    pub fn is_null(&self) -> bool {
        matches!(self, RuntimeCausalContext::Null)
    }
}

/// Daemon-local runtime invocation record.
///
/// What this is: an adapter record used by daemon schedule/loop/kernel
/// paths while they are being moved to Axon-native Invocation.
///
/// What this is not: the protocol Invocation primitive, a signing
/// source, or a canonical byte-layout definition. Call
/// `runtime_invocation_id` to derive the daemon-local session key; that
/// key is version-pinned and is not the Axon canonical invocation
/// identity (the wire envelope is built on the dispatch path).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeInvocation {
    pub caller: Ura,
    pub callee: Ura,
    pub ability: AbilityName,
    pub subject: Ura,
    /// 16-byte caller-generated nonce, hex-encoded. v1 generates via
    /// `fresh_nonce_hex()` at Control-layer admission time.
    pub nonce_hex: String,
    pub causal_context: RuntimeCausalContext,
    pub args: Value,
    /// Caller-produced signature over canonical bytes. Optional only for
    /// daemon-local system calls; public transport callers must provide it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_signature: Option<Vec<u8>>,
}

impl RuntimeInvocation {
    pub fn try_new(
        caller: Ura,
        callee: Ura,
        ability: AbilityName,
        subject: Ura,
        causal_context: RuntimeCausalContext,
        args: Value,
    ) -> Result<Self, RuntimeInvocationError> {
        let invocation = Self {
            caller,
            callee,
            ability,
            subject,
            nonce_hex: fresh_nonce_hex(),
            causal_context,
            args,
            caller_signature: None,
        };
        invocation.validate()?;
        Ok(invocation)
    }

    pub fn validate(&self) -> Result<(), RuntimeInvocationError> {
        validate_ura_field("caller", &self.caller)?;
        validate_ura_field("callee", &self.callee)?;
        validate_ura_field("subject", &self.subject)?;
        validate_ability_name(&self.ability)?;
        validate_nonce_hex(&self.nonce_hex)?;
        Ok(())
    }

    /// Encode the record into stable bytes for the daemon-local session
    /// key (see [`runtime_invocation_id`] and [`SESSION_KEY_ENCODING_VERSION`]).
    ///
    /// These bytes are NOT the Axon canonical invocation identity: the key
    /// is version-pinned on purpose so the same logical invocation maps to
    /// one session regardless of which descriptor version the dispatch
    /// path later negotiates. The wire-canonical envelope is built
    /// separately on the dispatch path, where it is bound to the
    /// registered descriptor version. The nonce — not the descriptor
    /// version — is what distinguishes one invocation from another here.
    ///
    /// The adapter intentionally accepts only `RuntimeCausalContext::Null`.
    /// Legacy scalar/list/merkle variants do not carry enough receipt
    /// material to build Axon `ReceiptRef`s, so accepting them would
    /// create a false proof of canonical equivalence.
    fn session_key_bytes(&self) -> Result<Vec<u8>, RuntimeInvocationError> {
        self.validate()?;
        let nonce = decode_nonce_hex(&self.nonce_hex)?;
        let args_bytes =
            serde_json::to_vec(&self.args).map_err(RuntimeInvocationError::ArgsJson)?;
        let causal_tag = match &self.causal_context {
            RuntimeCausalContext::Null => "none",
            RuntimeCausalContext::Scalar { .. } => {
                return Err(RuntimeInvocationError::LegacyCausalContext(
                    "scalar prior_invocation_id lacks receipt hash and receipt URA",
                ));
            }
            RuntimeCausalContext::List { .. } => {
                return Err(RuntimeInvocationError::LegacyCausalContext(
                    "list prior_invocation_ids lack receipt hashes and receipt URAs",
                ));
            }
            RuntimeCausalContext::Merkle { .. } => {
                return Err(RuntimeInvocationError::LegacyCausalContext(
                    "merkle_root lacks Axon proof URA",
                ));
            }
        };

        let mut out = Vec::new();
        out.extend_from_slice(b"easynet.runtime-invocation.session-key\0");
        push_session_key_field(&mut out, "version", SESSION_KEY_ENCODING_VERSION.as_bytes());
        push_session_key_field(&mut out, "caller", self.caller.as_bytes());
        push_session_key_field(&mut out, "callee", self.callee.as_bytes());
        push_session_key_field(&mut out, "ability", self.ability.as_bytes());
        push_session_key_field(&mut out, "subject", self.subject.as_bytes());
        push_session_key_field(&mut out, "nonce", &nonce);
        push_session_key_field(&mut out, "causal_context", causal_tag.as_bytes());
        push_session_key_field(&mut out, "args", &args_bytes);
        Ok(out)
    }

    pub fn axon_descriptor_bound_envelope(
        &self,
        descriptor_version: &str,
    ) -> Result<(easynet_axon::invocation::DescriptorBoundEnvelope, Vec<u8>), RuntimeInvocationError>
    {
        self.validate()?;
        let nonce = decode_nonce_hex(&self.nonce_hex)?;
        let args_bytes =
            serde_json::to_vec(&self.args).map_err(RuntimeInvocationError::ArgsJson)?;
        let causal_context = match &self.causal_context {
            RuntimeCausalContext::Null => easynet_axon::invocation::CausalContext::None,
            RuntimeCausalContext::Scalar { .. } => {
                return Err(RuntimeInvocationError::LegacyCausalContext(
                    "scalar prior_invocation_id lacks receipt hash and receipt URA",
                ));
            }
            RuntimeCausalContext::List { .. } => {
                return Err(RuntimeInvocationError::LegacyCausalContext(
                    "list prior_invocation_ids lack receipt hashes and receipt URAs",
                ));
            }
            RuntimeCausalContext::Merkle { .. } => {
                return Err(RuntimeInvocationError::LegacyCausalContext(
                    "merkle_root lacks Axon proof URA",
                ));
            }
        };
        let subject = easynet_axon::invocation::SubjectIdentity::new(
            self.subject.clone(),
            easynet_axon::invocation::UraProfile::EasynetStrictV2,
        );
        let ability = crate::daemon::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
            &self.callee,
            &self.ability,
            descriptor_version,
        )
        .map_err(|err| RuntimeInvocationError::AxonDescriptorBound(err.to_string()))?;
        let envelope = easynet_axon::invocation::DescriptorBoundEnvelope::from_parts(
            easynet_axon::invocation::DescriptorBoundEnvelopeParts {
                caller: easynet_axon::invocation::AgentIdentity::new(
                    self.caller.clone(),
                    easynet_axon::invocation::UraProfile::EasynetStrictV2,
                ),
                callee: easynet_axon::invocation::AgentIdentity::new(
                    self.callee.clone(),
                    easynet_axon::invocation::UraProfile::EasynetStrictV2,
                ),
                ability,
                subject,
                invocation_nonce: nonce,
                causal_context,
                args_bytes: &args_bytes,
            },
        )
        .map_err(|err| RuntimeInvocationError::AxonDescriptorBound(err.to_string()))?;
        Ok((envelope, args_bytes))
    }
}

/// Errors produced while validating or adapting a daemon-local runtime
/// invocation record.
#[derive(Debug, Error)]
pub enum RuntimeInvocationError {
    #[error("{field} URA must not be empty")]
    EmptyUra { field: &'static str },
    #[error("{field} URA must not contain leading or trailing whitespace")]
    UraHasSurroundingWhitespace { field: &'static str },
    #[error("{field} URA is invalid: {message}")]
    InvalidUra {
        field: &'static str,
        message: String,
    },
    #[error("ability must not be empty")]
    EmptyAbility,
    #[error("ability must not contain whitespace")]
    AbilityContainsWhitespace,
    #[error("nonce_hex is not hex: {0}")]
    NonceHex(#[from] hex::FromHexError),
    #[error("nonce_hex must encode exactly 16 bytes")]
    NonceLength,
    #[error("args JSON serialization failed: {0}")]
    ArgsJson(serde_json::Error),
    #[error("Axon descriptor-bound envelope failed: {0}")]
    AxonDescriptorBound(String),
    #[error("legacy causal context cannot be converted to Axon canonical bytes: {0}")]
    LegacyCausalContext(&'static str),
}

fn validate_ura_field(field: &'static str, value: &str) -> Result<(), RuntimeInvocationError> {
    if value.is_empty() {
        return Err(RuntimeInvocationError::EmptyUra { field });
    }
    if value.trim() != value {
        return Err(RuntimeInvocationError::UraHasSurroundingWhitespace { field });
    }
    crate::core::ura::parse_ura(value).map_err(|e| RuntimeInvocationError::InvalidUra {
        field,
        message: e.to_string(),
    })?;
    Ok(())
}

fn validate_ability_name(value: &str) -> Result<(), RuntimeInvocationError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(RuntimeInvocationError::EmptyAbility);
    }
    if value.chars().any(char::is_whitespace) {
        return Err(RuntimeInvocationError::AbilityContainsWhitespace);
    }
    Ok(())
}

fn validate_nonce_hex(value: &str) -> Result<(), RuntimeInvocationError> {
    let raw = decode_nonce_hex(value)?;
    if raw.len() != 16 {
        return Err(RuntimeInvocationError::NonceLength);
    }
    Ok(())
}

fn decode_nonce_hex(value: &str) -> Result<[u8; 16], RuntimeInvocationError> {
    let raw = hex::decode(value)?;
    if raw.len() != 16 {
        return Err(RuntimeInvocationError::NonceLength);
    }
    let mut nonce = [0u8; 16];
    nonce.copy_from_slice(&raw);
    Ok(nonce)
}

/// Fixed encoding version used solely to encode the daemon-local session key,
/// intentionally independent of the descriptor version the dispatch path
/// negotiates.
///
/// `runtime_invocation_id` must be derivable from a `RuntimeInvocation`
/// record alone, before any runtime is consulted, and must be stable
/// across descriptor-version changes so admission replay and session
/// idempotency keep mapping one logical invocation to one session. The
/// canonical wire envelope — which IS bound to the registered version —
/// is built separately on the dispatch path.
const SESSION_KEY_ENCODING_VERSION: &str = "runtime-invocation-session-v1";

fn push_session_key_field(out: &mut Vec<u8>, label: &str, bytes: &[u8]) {
    out.extend_from_slice(label.as_bytes());
    out.push(0);
    out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    out.extend_from_slice(bytes);
}

/// Compute `invocation_id = sha256(session_key_bytes(inv))`, hex-encoded.
///
/// This is a daemon-local session/receipt key for the adapter record —
/// NOT the Axon canonical invocation identity (see [`SESSION_KEY_ENCODING_VERSION`]).
/// It is deliberately a pure function of the record so the same logical
/// invocation always resolves to the same session key.
pub fn runtime_invocation_id(inv: &RuntimeInvocation) -> Result<String, RuntimeInvocationError> {
    let canonical = inv.session_key_bytes()?;
    Ok(hex::encode(easynet_axon::invocation::sha256(&canonical)))
}

/// Generate a fresh 16-byte nonce, hex-encoded.
///
/// Uses a UUID v4 random seed; collision-resistant enough for v1
/// admission dedup. Wire proto envelopes use `ProtoEnvelope`, which
/// emits a binary 16-byte nonce directly.
pub fn fresh_nonce_hex() -> String {
    hex::encode(easynet_axon::invocation::fresh_nonce())
}

/// Terminal state of a runtime invocation — the runtime decision that
/// closes the timeline (AXIOM §6.1 I2 terminal monotonic).
///
/// Axon `InvocationState` is the canonical terminal-state vocabulary.
/// This enum is the daemon-local receipt projection retained for older
/// schedule/kernel receipts; use [`TerminalState::from_axon_terminal`]
/// instead of hand-mapping success/failure at call sites.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TerminalState {
    Succeeded,
    Failed { reason: String },
    TimedOut { reason: String },
    Cancelled,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TerminalStateProjectionError {
    #[error("Axon invocation state `{state}` is not terminal")]
    NonTerminal { state: &'static str },
}

impl TerminalState {
    #[must_use]
    pub fn axon_terminal_state(&self) -> InvocationState {
        match self {
            Self::Succeeded => InvocationState::Completed,
            Self::Failed { .. } => InvocationState::Failed,
            Self::TimedOut { .. } => InvocationState::TimedOut,
            Self::Cancelled => InvocationState::Cancelled,
        }
    }

    pub fn from_axon_terminal(
        state: InvocationState,
        reason: Option<String>,
    ) -> Result<Self, TerminalStateProjectionError> {
        if !state.is_terminal() {
            return Err(TerminalStateProjectionError::NonTerminal {
                state: state.as_str(),
            });
        }
        Ok(match state {
            InvocationState::Completed => Self::Succeeded,
            InvocationState::Failed => Self::Failed {
                reason: reason.unwrap_or_else(|| "axon invocation failed".to_string()),
            },
            InvocationState::TimedOut => Self::TimedOut {
                reason: reason.unwrap_or_else(|| "axon invocation timed out".to_string()),
            },
            InvocationState::Cancelled => Self::Cancelled,
            InvocationState::Unspecified
            | InvocationState::Accepted
            | InvocationState::Admitted
            | InvocationState::Dispatched
            | InvocationState::Running => {
                unreachable!("is_terminal() gate rejects non-terminal states")
            }
        })
    }
}

impl TryFrom<InvocationState> for TerminalState {
    type Error = TerminalStateProjectionError;

    fn try_from(state: InvocationState) -> Result<Self, Self::Error> {
        Self::from_axon_terminal(state, None)
    }
}

/// Prior-receipt reference carried by a Receipt. Mirrors the four
/// causal-context shapes plus a zero-prior root.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PriorChain {
    None,
    Hash { prior_hash: String },
    Hashes { prior_hashes: Vec<String> },
    Root { prior_root: String },
}

/// One event in a runtime invocation's lifetime, replicated here for
/// compatibility with `daemon::execution::mission::timeline::TimelineEvent`. The
/// timeline layer remains authoritative; this struct only mirrors
/// the fields a Receipt needs to embed post-terminal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptEvent {
    pub sequence: i64,
    pub timestamp_unix_ms: i64,
    pub event_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
}

/// Daemon presentation projection of a terminal invocation outcome.
///
/// This is not a second signed receipt. When Axon admitted the invocation,
/// `terminal_receipt_hash` and `callee_signature` identify the already-verified
/// canonical terminal receipt. Pre-admission rejections have neither proof.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub invocation_id: String,
    pub terminal: TerminalState,
    pub events: Vec<ReceiptEvent>,
    pub prior: PriorChain,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_receipt_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callee_signature: Option<Vec<u8>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_invocation() -> RuntimeInvocation {
        RuntimeInvocation {
            caller: "easynet:///r/localhost/device/dev-a".into(),
            callee: "easynet:///r/localhost/device/dev-b".into(),
            ability: "observe.health".into(),
            subject: "easynet:///r/localhost/device/dev-b".into(),
            nonce_hex: "00112233445566778899aabbccddeeff".into(),
            causal_context: RuntimeCausalContext::Null,
            args: json!({}),
            caller_signature: None,
        }
    }

    #[test]
    fn invocation_id_is_stable_across_repeat_hash() {
        // Given the same inputs, `runtime_invocation_id` must return the
        // same hex digest — this is what makes it a valid system-
        // wide key.
        let inv = sample_invocation();
        let a = runtime_invocation_id(&inv).unwrap();
        let b = runtime_invocation_id(&inv).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 64, "sha256 hex digest is 64 chars");
    }

    #[test]
    fn invocation_id_changes_when_nonce_changes() {
        // Two distinct nonces on otherwise-identical Invocations must
        // produce distinct ids. This is the anti-replay property at
        // the content level; admission dedup enforces it at the
        // runtime level.
        let mut a = sample_invocation();
        let mut b = sample_invocation();
        a.nonce_hex = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into();
        b.nonce_hex = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into();
        assert_ne!(
            runtime_invocation_id(&a).unwrap(),
            runtime_invocation_id(&b).unwrap()
        );
    }

    #[test]
    fn terminal_state_projects_every_axon_terminal_state() {
        let completed = TerminalState::try_from(InvocationState::Completed).unwrap();
        assert_eq!(completed, TerminalState::Succeeded);
        assert_eq!(completed.axon_terminal_state(), InvocationState::Completed);

        let failed = TerminalState::from_axon_terminal(
            InvocationState::Failed,
            Some("handler failed".to_string()),
        )
        .unwrap();
        assert_eq!(
            failed,
            TerminalState::Failed {
                reason: "handler failed".to_string()
            }
        );
        assert_eq!(failed.axon_terminal_state(), InvocationState::Failed);

        let timed_out = TerminalState::from_axon_terminal(InvocationState::TimedOut, None).unwrap();
        assert_eq!(
            timed_out,
            TerminalState::TimedOut {
                reason: "axon invocation timed out".to_string()
            }
        );
        assert_eq!(timed_out.axon_terminal_state(), InvocationState::TimedOut);

        let cancelled = TerminalState::try_from(InvocationState::Cancelled).unwrap();
        assert_eq!(cancelled, TerminalState::Cancelled);
        assert_eq!(cancelled.axon_terminal_state(), InvocationState::Cancelled);
    }

    #[test]
    fn terminal_state_rejects_non_terminal_axon_states() {
        for state in [
            InvocationState::Unspecified,
            InvocationState::Accepted,
            InvocationState::Admitted,
            InvocationState::Dispatched,
            InvocationState::Running,
        ] {
            let err = TerminalState::try_from(state).expect_err("non-terminal must reject");
            assert_eq!(
                err,
                TerminalStateProjectionError::NonTerminal {
                    state: state.as_str()
                }
            );
        }
    }

    #[test]
    fn caller_signature_does_not_affect_invocation_id() {
        // The id hashes canonical bytes *excluding* the caller
        // signature so that attaching the v2 signature to a formerly
        // unsigned runtime invocation does not change its identity.
        let mut a = sample_invocation();
        let mut b = sample_invocation();
        b.caller_signature = Some(vec![1, 2, 3]);
        assert_eq!(
            runtime_invocation_id(&a).unwrap(),
            runtime_invocation_id(&b).unwrap()
        );
        a.caller_signature = Some(vec![9, 9, 9]);
        assert_eq!(
            runtime_invocation_id(&a).unwrap(),
            runtime_invocation_id(&b).unwrap()
        );
    }

    #[test]
    fn fresh_nonce_is_unique_per_call() {
        let a = fresh_nonce_hex();
        let b = fresh_nonce_hex();
        assert_ne!(a, b);
        assert_eq!(a.len(), 32, "16 bytes hex-encoded is 32 chars");
    }

    #[test]
    fn validate_rejects_non_ura_caller() {
        let mut inv = sample_invocation();
        inv.caller = "agent://self".into();
        let err = inv.validate().unwrap_err();
        assert!(format!("{err}").contains("caller URA is invalid"));
    }

    #[test]
    fn validate_rejects_ura_with_surrounding_whitespace() {
        let mut inv = sample_invocation();
        inv.caller = " easynet:///r/localhost/device/dev-a".into();
        let err = inv.validate().unwrap_err();
        assert!(matches!(
            err,
            RuntimeInvocationError::UraHasSurroundingWhitespace { field: "caller" }
        ));
    }

    #[test]
    fn try_new_builds_valid_invocation_with_fresh_nonce() {
        let inv = RuntimeInvocation::try_new(
            "easynet:///r/localhost/device/dev-a".into(),
            "easynet:///r/localhost/device/dev-b".into(),
            "skill.list".into(),
            "easynet:///r/localhost/device/dev-b".into(),
            RuntimeCausalContext::Null,
            json!({}),
        )
        .unwrap();
        assert_eq!(inv.nonce_hex.len(), 32);
        inv.validate().unwrap();
    }

    #[test]
    fn causal_context_serializes_with_tagged_kind() {
        let ctx = RuntimeCausalContext::Scalar {
            prior_invocation_id: "abc".into(),
        };
        let s = serde_json::to_string(&ctx).unwrap();
        assert!(s.contains("\"kind\":\"scalar\""));
        assert!(s.contains("\"prior_invocation_id\":\"abc\""));
    }

    #[test]
    fn legacy_scalar_causal_context_is_not_canonicalized() {
        let mut inv = sample_invocation();
        inv.causal_context = RuntimeCausalContext::Scalar {
            prior_invocation_id: "abc".into(),
        };
        let err = runtime_invocation_id(&inv).unwrap_err();
        assert!(
            matches!(err, RuntimeInvocationError::LegacyCausalContext(_)),
            "legacy causal context must fail before Axon canonicalization; got {err:?}"
        );
    }
}
