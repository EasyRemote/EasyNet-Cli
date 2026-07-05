// EasyNet CLI — host-stream shared contract
// ==========================================
//
// File: src/protocol/host_stream_contract.rs
// Description: Shared daemon SDK contract for host_stream frames, terminal
//              verification, and rolling output hashes.
//
// Protocol Responsibility
// -----------------------
// Own the EasyNet-Cli host-stream wire/hash contract used by daemon execution
// and SDK Host Binding projections. This module does not execute host code and
// does not define AbilityDescriptor identity.
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
// Daemon-owned Host Binding profile contract. EasyRemote/language hosts may
// own process warmth and Python function execution, but not these frame/hash
// semantics.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::daemon::ability::canonical_json_bytes;

pub(crate) const HOST_STREAM_FRAME_SCHEMA: &str = "host-stream-frame.schema.json";
pub(crate) const HOST_STREAM_HASH_ALGORITHM: &str =
    "sha256(prev_hash || seq_be || canonical_json(value))";

pub(crate) fn canonical_value_json(value: &Value) -> String {
    String::from_utf8(canonical_json_bytes(value))
        .expect("serde_json canonical object rendering is always UTF-8")
}

#[derive(Debug, Clone)]
pub(crate) struct HostStreamHashState {
    prev: [u8; 32],
    frames: u64,
    last_seq: Option<u64>,
}

impl HostStreamHashState {
    pub(crate) fn new() -> Self {
        Self {
            prev: Sha256::digest(b"").into(),
            frames: 0,
            last_seq: None,
        }
    }

    pub(crate) fn from_output_hash(
        output_hash: &str,
        frames: u64,
        last_seq: Option<u64>,
    ) -> Result<Self, HostStreamFailure> {
        let prev = parse_output_hash(output_hash)?;
        if frames == 0 && last_seq.is_some() {
            return Err(HostStreamFailure::new(
                HostStreamFailureKind::Protocol,
                "host-stream hash state cannot have last_seq when frames is zero".to_string(),
            ));
        }
        if frames > 0 && last_seq != Some(frames - 1) {
            return Err(HostStreamFailure::new(
                HostStreamFailureKind::Protocol,
                format!(
                    "host-stream hash state last_seq {:?} does not match frames {}",
                    last_seq, frames
                ),
            ));
        }
        Ok(Self {
            prev,
            frames,
            last_seq,
        })
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
        self.last_seq = Some(seq);
        Ok(())
    }

    pub(crate) fn output_hash(&self) -> String {
        format!("sha256:{}", hex::encode(self.prev))
    }

    pub(crate) fn frames(&self) -> u64 {
        self.frames
    }

    pub(crate) fn last_seq(&self) -> Option<u64> {
        self.last_seq
    }

    pub(crate) fn to_json(&self) -> Value {
        json!({
            "algorithm": HOST_STREAM_HASH_ALGORITHM,
            "output_hash": self.output_hash(),
            "frames": self.frames(),
            "last_seq": self.last_seq(),
        })
    }
}

impl Default for HostStreamHashState {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn parse_output_hash(output_hash: &str) -> Result<[u8; 32], HostStreamFailure> {
    let output_hash = output_hash.trim();
    let Some(hex_part) = output_hash.strip_prefix("sha256:") else {
        return Err(HostStreamFailure::new(
            HostStreamFailureKind::Protocol,
            "output_hash must use sha256:<64 lowercase hex> form".to_string(),
        ));
    };
    if hex_part.len() != 64
        || !hex_part
            .chars()
            .all(|ch| ch.is_ascii_digit() || ('a'..='f').contains(&ch))
    {
        return Err(HostStreamFailure::new(
            HostStreamFailureKind::Protocol,
            "output_hash must use sha256:<64 lowercase hex> form".to_string(),
        ));
    }
    let decoded = hex::decode(hex_part).map_err(|err| {
        HostStreamFailure::new(
            HostStreamFailureKind::Protocol,
            format!("output_hash hex decode failed: {err}"),
        )
    })?;
    let bytes: [u8; 32] = decoded.as_slice().try_into().map_err(|_| {
        HostStreamFailure::new(
            HostStreamFailureKind::Protocol,
            "output_hash must decode to exactly 32 bytes".to_string(),
        )
    })?;
    Ok(bytes)
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

pub(crate) fn decode_host_stream_request(envelope: &Value) -> Result<Value, HostStreamFailure> {
    let request = envelope
        .get("request")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            HostStreamFailure::new(
                HostStreamFailureKind::Protocol,
                "host-stream envelope missing request object".to_string(),
            )
        })?;
    let function = request
        .get("fn")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            HostStreamFailure::new(
                HostStreamFailureKind::Protocol,
                "host-stream request missing non-empty fn".to_string(),
            )
        })?;
    let args = request.get("args").ok_or_else(|| {
        HostStreamFailure::new(
            HostStreamFailureKind::Protocol,
            "host-stream request missing args".to_string(),
        )
    })?;
    let call_id = request
        .get("call_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            HostStreamFailure::new(
                HostStreamFailureKind::Protocol,
                "host-stream request missing non-empty call_id".to_string(),
            )
        })?;
    let caller = request
        .get("caller")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            HostStreamFailure::new(
                HostStreamFailureKind::Protocol,
                "host-stream request missing non-empty caller".to_string(),
            )
        })?;

    Ok(json!({
        "function": function,
        "args": args,
        "call_id": call_id,
        "caller": caller,
        "metadata": {
            "wire": "host_stream_request_v1",
            "source": "daemon_host_stream_contract",
        },
    }))
}

pub(crate) fn sdk_item_frame(seq: u64, value: Value) -> Value {
    json!({
        "frame_type": "item",
        "seq": seq,
        "value": value,
        "error": null,
        "terminal": null,
        "output_hash": null,
    })
}

pub(crate) fn sdk_error_frame(error: Value) -> Value {
    json!({
        "frame_type": "error",
        "seq": null,
        "value": null,
        "error": error,
        "terminal": null,
        "output_hash": null,
    })
}

pub(crate) fn sdk_terminal_frame(terminal: Value) -> Result<Value, HostStreamFailure> {
    let output_hash = terminal
        .get("output_hash")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            HostStreamFailure::new(
                HostStreamFailureKind::Protocol,
                "terminal missing string output_hash".to_string(),
            )
        })?;
    parse_output_hash(output_hash)?;
    let frames = terminal
        .get("frames")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            HostStreamFailure::new(
                HostStreamFailureKind::Protocol,
                "terminal missing u64 frames".to_string(),
            )
        })?;
    Ok(json!({
        "frame_type": "terminal",
        "seq": frames,
        "value": null,
        "error": null,
        "terminal": terminal,
        "output_hash": output_hash,
    }))
}

pub(crate) fn hash_state_from_json(
    value: &Value,
) -> Result<HostStreamHashState, HostStreamFailure> {
    let obj = value.as_object().ok_or_else(|| {
        HostStreamFailure::new(
            HostStreamFailureKind::Protocol,
            "host-stream hash state must be an object".to_string(),
        )
    })?;
    let algorithm = obj
        .get("algorithm")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            HostStreamFailure::new(
                HostStreamFailureKind::Protocol,
                "host-stream hash state missing algorithm".to_string(),
            )
        })?;
    if algorithm != HOST_STREAM_HASH_ALGORITHM {
        return Err(HostStreamFailure::new(
            HostStreamFailureKind::Protocol,
            format!(
                "host-stream hash state algorithm {algorithm:?} != {HOST_STREAM_HASH_ALGORITHM:?}"
            ),
        ));
    }
    let output_hash = obj
        .get("output_hash")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            HostStreamFailure::new(
                HostStreamFailureKind::Protocol,
                "host-stream hash state missing output_hash".to_string(),
            )
        })?;
    let frames = obj.get("frames").and_then(Value::as_u64).ok_or_else(|| {
        HostStreamFailure::new(
            HostStreamFailureKind::Protocol,
            "host-stream hash state missing u64 frames".to_string(),
        )
    })?;
    let last_seq = match obj.get("last_seq") {
        None | Some(Value::Null) => None,
        Some(value) => Some(value.as_u64().ok_or_else(|| {
            HostStreamFailure::new(
                HostStreamFailureKind::Protocol,
                "host-stream hash state last_seq must be u64 or null".to_string(),
            )
        })?),
    };
    HostStreamHashState::from_output_hash(output_hash, frames, last_seq)
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
    fn sdk_terminal_frame_requires_hash_and_frames() {
        let err = sdk_terminal_frame(json!({"frames": 1})).unwrap_err();
        assert_eq!(err.kind, HostStreamFailureKind::Protocol);
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
    fn decode_request_requires_current_daemon_envelope_fields() {
        let decoded = decode_host_stream_request(&json!({
            "request": {
                "fn": "weather.stream",
                "args": {"city": "Singapore"},
                "call_id": "call-1",
                "caller": "easynet:///r/example/user/alice"
            }
        }))
        .unwrap();
        assert_eq!(decoded["function"], "weather.stream");
        assert_eq!(decoded["args"]["city"], "Singapore");

        let err =
            decode_host_stream_request(&json!({"request": {"fn": "weather.stream"}})).unwrap_err();
        assert_eq!(err.kind, HostStreamFailureKind::Protocol);
    }

    #[test]
    fn hash_state_json_round_trips_current_state() {
        let mut state = HostStreamHashState::new();
        state.fold_item(0, &json!({"x": 1})).unwrap();
        let roundtrip = hash_state_from_json(&state.to_json()).unwrap();
        assert_eq!(roundtrip.output_hash(), state.output_hash());
        assert_eq!(roundtrip.frames(), 1);
        assert_eq!(roundtrip.last_seq(), Some(0));
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
