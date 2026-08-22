// EasyNet CLI — C ABI callback backpressure projection
// =====================================================
//
// File: src/ffi/invocation/backpressure.rs
// Description: Invocation C ABI callback-queue overflow projections.
//
// Protocol Responsibility
// -----------------------
// Own SDK-facing stream and bidirectional frame projection shapes that are not
// Axon canonical wire types. Axon remains the authority for stream/bidi
// protocol state machines; this module only projects binding-local lifecycle
// facts into stable Runtime Core DTOs.

use serde_json::{json, Value};

const BACKPRESSURE_WIRE_CODE: &str = "RESOURCE_EXHAUSTED";
const BACKPRESSURE_CANONICAL_CODE: &str = "ADMISSION_DENIED";
const BACKPRESSURE_RETRY: &str = "after_backoff";
const CALLBACK_QUEUE_OVERFLOW: &str = "callback_queue_overflow";

pub(crate) fn bidi_callback_backpressure_frame(sequence: u64, queue_capacity: usize) -> Value {
    json!({
        "ok": false,
        "kind": "error",
        "sequence": sequence,
        "terminal": false,
        "transport_terminal": true,
        "code": BACKPRESSURE_WIRE_CODE,
        "message": "C ABI bidi callback queue capacity exceeded",
        "error": runtime_backpressure_error("bidi", sequence, queue_capacity),
    })
}

fn runtime_backpressure_error(stage: &str, sequence: u64, queue_capacity: usize) -> Value {
    json!({
        "code": BACKPRESSURE_CANONICAL_CODE,
        "wire_code": BACKPRESSURE_WIRE_CODE,
        "stage": stage,
        "retry": BACKPRESSURE_RETRY,
        "retryable": true,
        "message": "Runtime callback queue backpressure bound exceeded",
        "details": {
            "reason": CALLBACK_QUEUE_OVERFLOW,
            "dropped_sequence": sequence,
            "queue_capacity": queue_capacity,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bidi_backpressure_frame_is_transport_terminal_not_runtime_terminal() {
        let frame = bidi_callback_backpressure_frame(3, 32);

        assert_eq!(frame["ok"], false);
        assert_eq!(frame["kind"], "error");
        assert_eq!(frame["sequence"], 3);
        assert_eq!(frame["terminal"], false);
        assert_eq!(frame["transport_terminal"], true);
        assert_eq!(frame["error"]["code"], "ADMISSION_DENIED");
        assert_eq!(frame["error"]["stage"], "bidi");
        assert_eq!(frame["error"]["details"]["dropped_sequence"], 3);
        assert_eq!(frame["error"]["details"]["queue_capacity"], 32);
    }
}
