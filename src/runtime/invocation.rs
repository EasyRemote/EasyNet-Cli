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

/// Unified Resource Address.
///
/// URAs are canonical `easynet:///r/<realm>/...` addresses. New code
/// should construct them through `crate::ura` builders and validate
/// externally supplied values with `crate::ura::parse_ura`. Plain
/// node ids, agent ids, or ad-hoc route fragments are not valid
/// Invocation URAs.
pub type Ura = String;

/// Ability member-call name.
///
/// This is the registry function name (`<agent>.<verb>`,
/// `device.skill.list`, `federation.resolve_key`, ...). The route or
/// resource URA that locates a published ability is carried by the
/// envelope/registry layers, not by this dispatch key.
pub type AbilityName = String;

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
    pub ability: AbilityName,
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
    pub fn try_new(
        caller: Ura,
        callee: Ura,
        ability: AbilityName,
        subject: Ura,
        causal_context: CausalContext,
        args: Value,
    ) -> anyhow::Result<Self> {
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

    pub fn validate(&self) -> anyhow::Result<()> {
        validate_ura_field("caller", &self.caller)?;
        validate_ura_field("callee", &self.callee)?;
        validate_ura_field("subject", &self.subject)?;
        validate_ability_name(&self.ability)?;
        validate_nonce_hex(&self.nonce_hex)?;
        Ok(())
    }

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

fn validate_ura_field(field: &str, value: &str) -> anyhow::Result<()> {
    let value = value.trim();
    if value.is_empty() {
        anyhow::bail!("{field} URA must not be empty");
    }
    crate::ura::parse_ura(value).map_err(|e| anyhow::anyhow!("{field} URA is invalid: {e}"))?;
    Ok(())
}

fn validate_ability_name(value: &str) -> anyhow::Result<()> {
    let value = value.trim();
    if value.is_empty() {
        anyhow::bail!("ability must not be empty");
    }
    if value.chars().any(char::is_whitespace) {
        anyhow::bail!("ability must not contain whitespace");
    }
    Ok(())
}

fn validate_nonce_hex(value: &str) -> anyhow::Result<()> {
    let raw = hex::decode(value).map_err(|e| anyhow::anyhow!("nonce_hex is not hex: {e}"))?;
    if raw.len() != 16 {
        anyhow::bail!("nonce_hex must encode exactly 16 bytes");
    }
    Ok(())
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
/// Uses a UUID v4 random seed; collision-resistant enough for v1
/// admission dedup. Wire proto envelopes use `ProtoEnvelope`, which
/// emits a binary 16-byte nonce directly.
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
            caller: "easynet:///r/localhost/device/dev-a".into(),
            callee: "easynet:///r/localhost/device/dev-b".into(),
            ability: "observe.health".into(),
            subject: "easynet:///r/localhost/device/dev-b".into(),
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
    fn validate_rejects_non_ura_caller() {
        let mut inv = sample_invocation();
        inv.caller = "agent://self".into();
        let err = inv.validate().unwrap_err();
        assert!(format!("{err}").contains("caller URA is invalid"));
    }

    #[test]
    fn try_new_builds_valid_invocation_with_fresh_nonce() {
        let inv = Invocation::try_new(
            "easynet:///r/localhost/device/dev-a".into(),
            "easynet:///r/localhost/device/dev-b".into(),
            "device.skill.list".into(),
            "easynet:///r/localhost/device/dev-b".into(),
            CausalContext::Null,
            json!({}),
        )
        .unwrap();
        assert_eq!(inv.nonce_hex.len(), 32);
        inv.validate().unwrap();
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
