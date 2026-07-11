// EasyNet CLI — host-stream executor contract
// ===========================================
//
// File: src/daemon/execution/mission/executors/host_stream/contract.rs
// Description: Executor-local host_stream frame decoding, terminal
//              verification, and rolling output hashes.
//
// Protocol Responsibility
// -----------------------
// Own the host-stream wire/hash contract used by this daemon executor. This
// module does not execute host code and does not define AbilityDescriptor
// identity.
//
// Implementation Approach
// -----------------------
// Reuse the daemon ability canonical JSON helper, then fold
// `sha256(prev_hash || seq_be || canonical_json(value))` over item values in
// strict sequence order.
//
// Usage Contract
// --------------
// Callers supply explicit sequence numbers. `fold_item` rejects gaps and
// reorders instead of inferring sequence. Terminal verification requires both
// frame count and final output hash.
//
// Architectural Position
// ----------------------
// Daemon-owned mission executor detail. EasyRemote may own process warmth and
// Python function execution, but not these frame/hash semantics.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::daemon::ability::canonical_json_bytes;

#[derive(Debug, Clone)]
pub(crate) struct HostStreamHashState {
    prev: [u8; 32],
    frames: u64,
}

impl HostStreamHashState {
    pub(crate) fn new() -> Self {
        Self {
            prev: Sha256::digest(b"").into(),
            frames: 0,
        }
    }

    pub(crate) fn fold_item(&mut self, seq: u64, value: &Value) -> Result<(), HostStreamFailure> {
        if seq != self.frames {
            return Err(HostStreamFailure::new(
                HostStreamFailureKind::StreamTruncated,
                format!("frame reorder/gap: expected seq {}, got {seq}", self.frames),
            ));
        }
        let mut hasher = Sha256::new();
        hasher.update(self.prev);
        hasher.update(seq.to_be_bytes());
        hasher.update(canonical_json_bytes(value));
        self.prev = hasher.finalize().into();
        self.frames += 1;
        Ok(())
    }

    pub(crate) fn output_hash(&self) -> String {
        format!("sha256:{}", hex::encode(self.prev))
    }
}

pub(crate) fn verify_terminal(
    terminal: &Value,
    rolling: &HostStreamHashState,
    frames_seen: u64,
) -> Result<(), HostStreamFailure> {
    let frames = terminal
        .get("frames")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            HostStreamFailure::new(
                HostStreamFailureKind::Protocol,
                "terminal missing u64 frames".to_string(),
            )
        })?;
    if frames != frames_seen {
        return Err(HostStreamFailure::new(
            HostStreamFailureKind::StreamTruncated,
            format!("terminal frame count {frames} != frames received {frames_seen}"),
        ));
    }

    let declared = terminal
        .get("output_hash")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            HostStreamFailure::new(
                HostStreamFailureKind::Protocol,
                "terminal missing string output_hash".to_string(),
            )
        })?;
    let computed = rolling.output_hash();
    if declared != computed {
        return Err(HostStreamFailure::new(
            HostStreamFailureKind::StreamTruncated,
            format!("output_hash mismatch: host {declared} != computed {computed}"),
        ));
    }
    Ok(())
}

#[derive(Debug)]
pub(crate) enum HostFrame<'a> {
    StreamItem { seq: u64, item: &'a Value },
    Terminal(&'a Value),
    Error(&'a Value),
}

pub(crate) fn decode_host_frame(frame: &Value) -> Result<HostFrame<'_>, HostStreamFailure> {
    let has_item = frame.get("stream_item").is_some();
    let has_terminal = frame.get("terminal").is_some();
    let has_error = frame.get("error").is_some();
    let kinds = usize::from(has_item) + usize::from(has_terminal) + usize::from(has_error);
    if kinds != 1 {
        return Err(HostStreamFailure::new(
            HostStreamFailureKind::Protocol,
            "frame must contain exactly one of stream_item/terminal/error".to_string(),
        ));
    }

    if let Some(item) = frame.get("stream_item") {
        let seq = frame.get("seq").and_then(Value::as_u64).ok_or_else(|| {
            HostStreamFailure::new(
                HostStreamFailureKind::Protocol,
                "stream_item missing u64 seq".into(),
            )
        })?;
        return Ok(HostFrame::StreamItem { seq, item });
    }
    if let Some(terminal) = frame.get("terminal") {
        return Ok(HostFrame::Terminal(terminal));
    }
    let error = frame
        .get("error")
        .expect("exactly one host-stream frame kind was already checked");
    Ok(HostFrame::Error(error))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HostStreamFailureKind {
    HostUnreachable,
    StreamTruncated,
    Protocol,
    Internal,
    Host(String),
}

impl HostStreamFailureKind {
    pub(crate) fn as_str(&self) -> &str {
        match self {
            HostStreamFailureKind::HostUnreachable => "HOST_UNREACHABLE",
            HostStreamFailureKind::StreamTruncated => "STREAM_TRUNCATED",
            HostStreamFailureKind::Protocol => "PROTOCOL",
            HostStreamFailureKind::Internal => "INTERNAL",
            HostStreamFailureKind::Host(kind) => kind.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostStreamFailure {
    pub(crate) kind: HostStreamFailureKind,
    pub(crate) message: String,
}

impl HostStreamFailure {
    pub(crate) fn new(kind: HostStreamFailureKind, message: String) -> Self {
        Self { kind, message }
    }

    pub(crate) fn from_host_error(error: &Value) -> Self {
        let kind = error
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("HOST_ERROR")
            .to_string();
        Self {
            kind: HostStreamFailureKind::Host(kind),
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("host reported an error")
                .to_string(),
        }
    }

    pub(crate) fn error_frame(&self) -> Value {
        json!({
            "error": {
                "kind": self.kind.as_str(),
                "message": self.message,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rolling_hash_is_order_sensitive_and_deterministic() {
        let mut a = HostStreamHashState::new();
        a.fold_item(0, &json!({"x": 1})).unwrap();
        a.fold_item(1, &json!({"y": 2})).unwrap();

        let mut b = HostStreamHashState::new();
        b.fold_item(0, &json!({"x": 1})).unwrap();
        b.fold_item(1, &json!({"y": 2})).unwrap();
        assert_eq!(a.output_hash(), b.output_hash());

        let mut c = HostStreamHashState::new();
        c.fold_item(0, &json!({"y": 2})).unwrap();
        c.fold_item(1, &json!({"x": 1})).unwrap();
        assert_ne!(a.output_hash(), c.output_hash());
    }

    #[test]
    fn rolling_hash_uses_shared_key_sorted_json() {
        let mut a = HostStreamHashState::new();
        a.fold_item(0, &json!({"b": 2, "a": {"d": 4, "c": 3}}))
            .unwrap();

        let mut b = HostStreamHashState::new();
        b.fold_item(0, &json!({"a": {"c": 3, "d": 4}, "b": 2}))
            .unwrap();

        assert_eq!(a.output_hash(), b.output_hash());
    }

    #[test]
    fn rolling_hash_rejects_gap_or_reorder() {
        let mut rolling = HostStreamHashState::new();
        let err = rolling.fold_item(1, &json!({"x": 1})).unwrap_err();
        assert_eq!(err.kind, HostStreamFailureKind::StreamTruncated);
    }

    #[test]
    fn verify_terminal_accepts_matching_hash_and_count() {
        let mut rolling = HostStreamHashState::new();
        rolling.fold_item(0, &json!({"x": 1})).unwrap();
        let terminal = json!({"output_hash": rolling.output_hash(), "frames": 1});

        verify_terminal(&terminal, &rolling, 1).expect("matching terminal must verify");
    }

    #[test]
    fn verify_terminal_rejects_output_hash_mismatch() {
        let mut rolling = HostStreamHashState::new();
        rolling.fold_item(0, &json!({"x": 1})).unwrap();
        let err = verify_terminal(
            &json!({"frames": 1, "output_hash": "sha256:deadbeef"}),
            &rolling,
            1,
        )
        .unwrap_err();
        assert_eq!(err.kind, HostStreamFailureKind::StreamTruncated);
    }

    #[test]
    fn decode_host_frame_rejects_mixed_frame_kinds() {
        let err = decode_host_frame(&json!({
            "seq": 0,
            "stream_item": {"x": 1},
            "terminal": {"frames": 1, "output_hash": "sha256:deadbeef"}
        }))
        .unwrap_err();
        assert_eq!(err.kind, HostStreamFailureKind::Protocol);
    }

    #[test]
    fn host_error_frame_preserves_kind_and_message() {
        let failure = HostStreamFailure::from_host_error(&json!({
            "kind": "BOOM",
            "message": "it broke"
        }));
        let frame = failure.error_frame();
        assert_eq!(frame["error"]["kind"], "BOOM");
        assert_eq!(frame["error"]["message"], "it broke");
    }
}
