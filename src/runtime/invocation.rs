// EasyNet CLI — Invocation (system-level unit of execution)
// ==========================================================
//
// File: src/runtime/invocation.rs
// Description: The seven-parameter Invocation structure that AXIOM §2
//              pins as the sole protocol primitive, plus the Receipt
//              terminal record and the causal-context shapes.
//
// Why this module is the root of runtime types
// --------------------------------------------
// The plan v10.2–v10.5 collapses every execution path onto a single
// syntactic and semantic object: Invocation. Every entry into the
// runtime — Client FFI, schedule tick, loop controller, permission
// admission, Axon inbound task — is first lifted to an Invocation
// whose `invocation_id` is then the system-wide unique key the rest of
// the runtime indexes by (IPC request_id, KernelApi call id, Axon
// `send_a2a_task` id — all one id).
//
// v1 vs v2 signature status
// -------------------------
// `caller_signature` and `callee_signature` are Option<Vec<u8>> and
// always None in v1 (AXIOM §6.3 signed-invocation is not enabled;
// federation trust still rides Axon mTLS). The fields exist on the
// wire so v2 can start populating them without a schema migration.
// See docs/design/formal-model-v1.md for the C1/C2 non-repudiation
// invariants that these fields will eventually satisfy.
//
// v1 classification (v10.5 R1)
// ----------------------------
// The type system here describes *structural* invariants (shape of an
// Invocation, shape of a Receipt, shape of a CausalContext). The
// *semantic* invariants S1–S4 (receipts as runtime inputs) are
// explicitly not in scope for v1 — v1 is a record system, not a
// computation system. docs/design/formal-model-v1.md pins this.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Unified Resource Address. Placeholder-typed as an opaque string in
/// v1 — the URA scheme (`easynet://...`) is itself under AXIOM §3 and
/// has no CLI-side construction rule yet. Callers may stuff a plain
/// node_id or agent_id here; future validation will tighten to the
/// URA grammar.
pub type Ura = String;

/// Ability URI. In v1 this is the same `<agent>.<verb>` or
/// `system.<feature>.<verb>` string the ability registry already
/// consumes; the `easynet://` URI prefix is added when signed
/// invocation ships in v2.
pub type AbilityUri = String;

/// The four shapes `causal_context` may take per AXIOM §2.x.
///
/// - `Null`   — a freshly-initiated Invocation with no prior receipt
///              in its causal past (e.g. a user-initiated Client FFI
///              call that did not cite any prior receipt).
/// - `Scalar` — a single prior invocation's receipt hash. This is the
///              common shape for "B ran because A completed".
/// - `List`   — multiple prior receipts forming a set causal parent.
/// - `Merkle` — a Merkle root over a large set of prior receipts,
///              used when list cardinality would blow the envelope.
///
/// v1 emits `Null` and `Scalar` only (schedule tick / loop controller
/// / permission admission all have at most one prior receipt to cite).
/// `List` and `Merkle` variants exist on the wire so v2 causal
/// scheduling can populate them without a schema migration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CausalContext {
    Null,
    Scalar { prior_invocation_id: String },
    List { prior_invocation_ids: Vec<String> },
    Merkle { merkle_root: String },
}

impl CausalContext {
    pub fn is_null(&self) -> bool {
        matches!(self, CausalContext::Null)
    }
}

/// AXIOM §2 seven-parameter Invocation.
///
/// The fields mirror the AXIOM tuple verbatim so audit tooling that
/// cross-references a runtime trace with the paper can locate each
/// parameter without translation. `args` is typed as `Value` in v1
/// because the ability input schema is still JSON-shaped; v2 will
/// carry proto-encoded bytes here once schemas/ is wired end-to-end.
///
/// `invocation_id` is not stored on the struct: it is derived from
/// canonical bytes (see `invocation_id_of`). Keeping it out of the
/// struct prevents a caller from forging an id that disagrees with
/// the content — the id is a function of the Invocation, not an
/// attribute.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Invocation {
    pub caller: Ura,
    pub callee: Ura,
    pub ability: AbilityUri,
    pub subject: Ura,
    /// 16-byte caller-generated nonce, hex-encoded. v1 generates via
    /// `fresh_nonce_hex()` at Control-layer admission time.
    pub nonce_hex: String,
    pub causal_context: CausalContext,
    pub args: Value,
    /// Caller-produced signature over canonical bytes. v1 = None;
    /// v2 mandatory (AXIOM §6.3 C1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_signature: Option<Vec<u8>>,
}

impl Invocation {
    /// Canonical byte representation used for hashing. Order-stable by
    /// serde_json's `to_vec` on the named struct, which is
    /// insertion-order for struct fields and sorted-by-key for
    /// `Value` sub-trees. For v1 this is enough; v2 protobuf mapping
    /// will pin canonical bytes at the wire level.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        // serialize without the optional caller_signature field so a
        // later v2 signing pass does not rehash a different payload.
        let mut cloned = self.clone();
        cloned.caller_signature = None;
        serde_json::to_vec(&cloned).unwrap_or_default()
    }
}

/// Compute `invocation_id = sha256(canonical_bytes(inv))`, hex-encoded.
///
/// This is the system-wide unique id for one Invocation. The IPC layer
/// reuses it as `request_id`; Kernel::invoke keys its in-flight table
/// on it; Axon `send_a2a_task` carries it as `task_id` when the
/// dispatch is remote. Three names in the codebase, one id.
pub fn invocation_id_of(inv: &Invocation) -> String {
    let mut h = Sha256::new();
    h.update(inv.canonical_bytes());
    hex::encode(h.finalize())
}

/// Generate a fresh 16-byte nonce, hex-encoded.
///
/// Uses the process pid + wall-clock nanos + a uuid v4 seed.
/// Collision-resistant enough for v1 admission dedup (5-minute
/// window, single-tenant). v2 may switch to an OS-level RNG.
pub fn fresh_nonce_hex() -> String {
    let uuid_bytes = uuid::Uuid::new_v4().into_bytes();
    hex::encode(uuid_bytes)
}

/// Terminal state of an Invocation — the runtime decision that
/// closes the timeline (AXIOM §6.1 I2 terminal monotonic).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TerminalState {
    Succeeded,
    Failed { reason: String },
    Cancelled,
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

/// One event in an Invocation's lifetime, replicated here for
/// compatibility with `runtime::timeline::TimelineEvent`. The
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

/// AXIOM §6.1 I3 Receipt — the durable record that an Invocation
/// terminated. In v1 the callee_signature field is always `None`
/// (I3 integrity holds via the in-process events hash; non-
/// repudiation C2 lands with v2 signed invocation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub invocation_id: String,
    pub terminal: TerminalState,
    pub events: Vec<ReceiptEvent>,
    pub prior: PriorChain,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callee_signature: Option<Vec<u8>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_invocation() -> Invocation {
        Invocation {
            caller: "easynet://nodes/a".into(),
            callee: "easynet://nodes/b".into(),
            ability: "observe.health".into(),
            subject: "easynet://nodes/b".into(),
            nonce_hex: "00112233445566778899aabbccddeeff".into(),
            causal_context: CausalContext::Null,
            args: json!({}),
            caller_signature: None,
        }
    }

    #[test]
    fn invocation_id_is_stable_across_repeat_hash() {
        // Given the same inputs, `invocation_id_of` must return the
        // same hex digest — this is what makes it a valid system-
        // wide key.
        let inv = sample_invocation();
        let a = invocation_id_of(&inv);
        let b = invocation_id_of(&inv);
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
        assert_ne!(invocation_id_of(&a), invocation_id_of(&b));
    }

    #[test]
    fn caller_signature_does_not_affect_invocation_id() {
        // The id hashes canonical bytes *excluding* the caller
        // signature so that attaching the v2 signature to a formerly
        // unsigned Invocation does not change its identity.
        let mut a = sample_invocation();
        let mut b = sample_invocation();
        b.caller_signature = Some(vec![1, 2, 3]);
        assert_eq!(invocation_id_of(&a), invocation_id_of(&b));
        a.caller_signature = Some(vec![9, 9, 9]);
        assert_eq!(invocation_id_of(&a), invocation_id_of(&b));
    }

    #[test]
    fn fresh_nonce_is_unique_per_call() {
        let a = fresh_nonce_hex();
        let b = fresh_nonce_hex();
        assert_ne!(a, b);
        assert_eq!(a.len(), 32, "16 bytes hex-encoded is 32 chars");
    }

    #[test]
    fn causal_context_serializes_with_tagged_kind() {
        let ctx = CausalContext::Scalar {
            prior_invocation_id: "abc".into(),
        };
        let s = serde_json::to_string(&ctx).unwrap();
        assert!(s.contains("\"kind\":\"scalar\""));
        assert!(s.contains("\"prior_invocation_id\":\"abc\""));
    }
}
