// EasyNet CLI — `<self>.dispatch` / `<self>.upstream_invoke` frame schemas
// =========================================================================
//
// File: src/runtime/agents/self_dispatch_frames.rs
//
// What this module is
// -------------------
// Wire-shape of the JSON payloads carried inside `axon.v1.BinaryChunk.data`
// for the two transport-plane abilities introduced by RFC-003:
//
//   * `<self>.dispatch`        — long-lived reverse channel from a device
//                                to its hub. Hub pushes Dispatch frames
//                                down; device replies with Result frames.
//   * `<self>.upstream_invoke` — per-invoke short-lived stream from a
//                                device to its hub. Device sends a single
//                                Request frame on EnvelopeOpen; hub returns
//                                Chunk* + a terminal Result.
//
// Why JSON inside `BinaryChunk`
// -----------------------------
// We deliberately do NOT introduce a new `.proto` file. The carrier is
// `axon.v1.Invocation::InvokeBidi` verbatim — same RPC surface every other
// ability uses. By tunnelling our frame typing as JSON inside the existing
// `BinaryChunk.data` bytes we:
//
//   * Reuse axon's signed-envelope / admission / membership / delegation
//     gates without writing a single line of new gate code.
//   * Avoid a proto-compiler regen across the seven SDKs that consume
//     axon's wire types (Rust / Go / Python / Java / Kotlin / TypeScript /
//     Swift). The MVP validation in `EasyNet-Federation-MVP` proves this
//     is byte-faithful and structurally tight.
//   * Keep the schema iterable in plain Rust enums — adding a new frame
//     variant is a serde change, not a protoc invocation.
//
// Validation provenance
// ---------------------
// Every variant defined here was exercised by `EasyNet-Federation-MVP`'s
// `e2e-test.sh` 11-case suite (echo, ping, 50-way concurrency, 1 MiB
// binary, audio bursts, video frames, agent.method() generic invoke,
// SIGKILL detect, hub crash). The Rust enums are byte-for-byte the same
// ones the MVP `common/src/lib.rs` declares; the move into production
// changes nothing about the on-the-wire payload.
//
// Architectural position
// ----------------------
// This is a leaf module — no dependencies on the rest of the runtime.
// Both the dispatch acceptor (`self_dispatch.rs`, hub-mode handler) and
// the federation_client rewrite (PR-4) refer here for typed (de)serdes.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde::{Deserialize, Serialize};

/// Ability name for the long-lived device→hub reverse channel.
pub const ABILITY_DISPATCH: &str = "<self>.dispatch";

/// Ability name for the per-invoke device→hub upstream stream.
pub const ABILITY_UPSTREAM_INVOKE: &str = "<self>.upstream_invoke";

/// Stream id used by every BinaryChunk on the two abilities. The
/// EnvelopeOpen always declares exactly one StreamDescriptor with this
/// id; per-frame `stream_id` always references it. Matches the MVP's
/// invariant so the production wire is bytewise interchangeable with
/// MVP-emitted traffic for cross-validation.
pub const SELF_STREAM_ID: u32 = 0;

// ─── <self>.dispatch frames ────────────────────────────────────────────────

/// Frames a device sends *up* the dispatch stream toward the hub.
///
/// The `Hello` variant is always the first payload — it lands inside
/// `EnvelopeOpen.initial_args` in frame 0. Subsequent frames are
/// `Result` replies to hub-pushed Dispatch commands and ride
/// `BinaryChunk` frames.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DispatchUp {
    /// First payload (carried in `EnvelopeOpen.initial_args`). Tells the
    /// hub which device this stream represents. The hub uses
    /// `device_id` as the key in its presence registry; the value is
    /// the senderside of the down-direction `mpsc` channel.
    Hello { device_id: String },

    /// Reply to a hub-pushed Dispatch. `call_id` correlates back to
    /// the in-flight request the hub parked on a `oneshot`. Carried
    /// in subsequent BinaryChunk frames after frame 0.
    ///
    /// `terminal: true` says "no more frames for this call_id". The
    /// MVP only ever emits a single terminal Result per call; chunked
    /// streaming results are reserved for a follow-up RFC.
    Result {
        call_id: u64,
        payload: Vec<u8>,
        terminal: bool,
        error: Option<String>,
    },
}

/// Frames the hub sends *down* the dispatch stream toward the device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DispatchDown {
    /// "Run this ability locally and reply." Issued by the hub when
    /// some other device's `<self>.upstream_invoke` targeted this
    /// device. `ability` is the bare verb the device's local
    /// dispatcher will resolve against its registry; `args` is the
    /// caller-shaped argument bytes (typically JSON, but the field
    /// is opaque on this layer).
    Dispatch {
        call_id: u64,
        ability: String,
        args: Vec<u8>,
    },

    /// Operator- or hub-triggered shutdown notification. Reserved
    /// (the MVP does not exercise it); a device receiving Shutdown
    /// is expected to drain in-flight calls and reconnect.
    Shutdown {},
}

// ─── <self>.upstream_invoke frames ─────────────────────────────────────────

/// Frame 0 carried inside `EnvelopeOpen.initial_args` on a per-invoke
/// upstream stream. There is exactly one variant today — the schema is
/// an enum so a future versioned variant can be added without a new
/// ability name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UpstreamUp {
    /// "Forward this ability invocation to the device named by
    /// `subject_device`." The hub resolves the target by looking it
    /// up in its presence registry (the device's
    /// `<self>.dispatch` stream sender), allocates a `call_id`,
    /// pushes a `DispatchDown::Dispatch`, and parks awaiting the
    /// corresponding `DispatchUp::Result`.
    Request {
        subject_device: String,
        ability: String,
        args: Vec<u8>,
    },
}

/// Frames the hub sends *down* a per-invoke upstream stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UpstreamDown {
    /// Intermediate streaming chunk. Reserved for the chunked-result
    /// extension; the MVP issues only Result terminal frames today.
    Chunk { payload: Vec<u8> },

    /// Terminal frame for a per-invoke upstream stream. `error` is
    /// populated when the hub couldn't route (`subject_device` not in
    /// presence registry) or when the target device returned an error
    /// in its `DispatchUp::Result`.
    Result {
        payload: Vec<u8>,
        terminal: bool,
        error: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_up_hello_round_trip() {
        let f = DispatchUp::Hello {
            device_id: "dev-A".into(),
        };
        let bytes = serde_json::to_vec(&f).unwrap();
        let recovered: DispatchUp = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(f, recovered);
    }

    #[test]
    fn dispatch_up_result_round_trip() {
        let f = DispatchUp::Result {
            call_id: 42,
            payload: b"hello".to_vec(),
            terminal: true,
            error: None,
        };
        let bytes = serde_json::to_vec(&f).unwrap();
        let recovered: DispatchUp = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(f, recovered);
    }

    #[test]
    fn dispatch_down_dispatch_round_trip() {
        let f = DispatchDown::Dispatch {
            call_id: 99,
            ability: "echo".into(),
            args: vec![1, 2, 3],
        };
        let bytes = serde_json::to_vec(&f).unwrap();
        let recovered: DispatchDown = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(f, recovered);
    }

    #[test]
    fn upstream_up_request_round_trip() {
        let f = UpstreamUp::Request {
            subject_device: "dev-B".into(),
            ability: "shell.run".into(),
            args: b"{\"cmd\":\"ls\"}".to_vec(),
        };
        let bytes = serde_json::to_vec(&f).unwrap();
        let recovered: UpstreamUp = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(f, recovered);
    }

    #[test]
    fn upstream_down_result_round_trip() {
        let f = UpstreamDown::Result {
            payload: b"pong".to_vec(),
            terminal: true,
            error: None,
        };
        let bytes = serde_json::to_vec(&f).unwrap();
        let recovered: UpstreamDown = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(f, recovered);
    }

    #[test]
    fn upstream_down_error_round_trip() {
        let f = UpstreamDown::Result {
            payload: vec![],
            terminal: true,
            error: Some("target offline".into()),
        };
        let bytes = serde_json::to_vec(&f).unwrap();
        let recovered: UpstreamDown = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(f, recovered);
    }

    #[test]
    fn ability_name_constants_are_self_namespaced() {
        // The `<self>.` prefix is the documented marker for abilities
        // whose effective subject is whoever holds the registration
        // (the daemon itself). Catching a future rename here protects
        // the MVP's wire-compat promise.
        assert_eq!(ABILITY_DISPATCH, "<self>.dispatch");
        assert_eq!(ABILITY_UPSTREAM_INVOKE, "<self>.upstream_invoke");
    }
}
