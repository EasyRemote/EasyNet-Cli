// EasyNet CLI — Control-plane wire frames
// ========================================
//
// File: src/services/control/frames.rs
// Description: The length-prefixed JSON frame schema for the local
//              IPC control plane. A frame is one envelope (Invoke /
//              Subscribe / Cancel going in; Result / Frame / Terminal
//              / Error coming out). Callers write one envelope per
//              frame; the transport layer adds a 4-byte LE length
//              prefix.
//
// Canonical proto source
// ----------------------
// v10.5 R1 pins Protobuf as the schema source of truth. v1 does not
// yet generate these types from `schemas/control_plane.proto`;
// instead, the structs here mirror the message shapes the proto
// file will define. When schemas/ is wired end-to-end (follow-up
// commit inside PR-DAEMON), `prost-build` will replace these
// hand-rolled structs with generated ones and the serde-tag layout
// will match 1:1 so the JSON wire format stays stable across the
// cut-over. Until then, the serde `#[serde(tag = "type", rename_all =
// "snake_case")]` discriminator reproduces the proto oneof shape.
//
// Why serde_json::Value for args / value / frame
// ----------------------------------------------
// The envelope is ability-agnostic; the inner payload is whatever
// ability-specific schema the ability declares. v1 leaves those
// payloads as `serde_json::Value`; v2 swaps them for proto-encoded
// bytes once every feature PR's `.proto` file is in place.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One inbound envelope from the Client FFI. The discriminator is
/// `type`, matching the eventual proto oneof field name.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IncomingFrame {
    /// RPC-style ability call. Expect exactly one `Result` or
    /// `Error` envelope in response.
    ///
    /// `subject` carries the AXIOM envelope's subject URI (the
    /// resource the ability acts on, e.g. a camera resource URA for
    /// `camera.snapshot`). Older clients omit it; the daemon
    /// forwards it to `InvocationPlan.subject` when present so
    /// envelope-aware handlers see it via `EnvelopeContext`.
    Invoke {
        request_id: String,
        ability: String,
        #[serde(default)]
        args: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject: Option<String>,
    },
    /// Streaming ability call. Expect zero or more `Frame` envelopes
    /// followed by exactly one `Terminal` envelope.
    Subscribe {
        subscription_id: String,
        ability: String,
        #[serde(default)]
        args: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject: Option<String>,
    },
    /// Early-terminate an in-flight subscription. Server responds
    /// with a `Terminal` envelope on the matching subscription_id.
    Cancel { subscription_id: String },
    /// Open a bidirectional session against a registered bidi
    /// ability. The handler is invoked once with `args`; both
    /// directions stay open until the client sends `CloseBidi`,
    /// the handler ends, or the connection drops. Server emits zero
    /// or more `RecvBidi` frames followed by exactly one `Terminal`
    /// envelope.
    OpenBidi {
        session_id: String,
        ability: String,
        #[serde(default)]
        args: Value,
    },
    /// Push one client→handler frame onto an opened bidi session.
    /// Frames are delivered to the handler in client emission order
    /// (per-session FIFO). No response envelope per `SendBidi` —
    /// any reply rides on `RecvBidi`.
    SendBidi { session_id: String, frame: Value },
    /// Client-initiated close. Server drops the handler-input
    /// channel; the handler observes EOF, ends, and the forwarder
    /// emits `Terminal{done}`. Idempotent — a second `CloseBidi` for
    /// the same `session_id` is a silent no-op.
    CloseBidi { session_id: String },
}

/// One outbound envelope written back to the Client FFI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutgoingFrame {
    /// Single-shot RPC response. Matches an `Invoke` frame's
    /// `request_id`.
    ///
    /// `receipt_header` is the §A12 / §1.3 staging shape carrying
    /// `callee_agent_ura` / `signer_agent_ura` / signing model.
    /// Optional on the wire so older Clients that don't decode it
    /// tolerate the addition; present whenever the dispatch path
    /// can determine which Agent owned the ability (P4.8c onwards).
    /// When absent, callers default to "Selfsigned by the device-
    /// profile" — the historical behaviour pre-RFC.
    Result {
        request_id: String,
        value: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        receipt_header: Option<crate::runtime::hosted_receipt::HostedAgentReceiptHeader>,
    },
    /// One streaming frame. Matches a `Subscribe` frame's
    /// `subscription_id`.
    Frame {
        subscription_id: String,
        frame: Value,
    },
    /// Stream terminated. `reason` is a short machine-readable code
    /// ("done" / "cancelled" / "error:…"). Ability-specific error
    /// details, when present, come in a preceding `Error` envelope.
    Terminal {
        subscription_id: String,
        reason: String,
    },
    /// Bidi session terminated. Mirrors `Terminal` but carries
    /// `session_id` instead of `subscription_id` so the wire shape
    /// matches OpenBidi's correlation id and so the type system
    /// rejects accidentally closing a stream as a bidi (and vice
    /// versa). Per C-M3a §I2, **at most one TerminalBidi is ever
    /// emitted per session_id** — the cancel-path that flips the
    /// per-session `finalized` flag first wins.
    TerminalBidi { session_id: String, reason: String },
    /// Error envelope, used for Invoke and Subscribe failures.
    /// Exactly one of `request_id` / `subscription_id` is populated
    /// depending on which inbound frame caused the error.
    Error {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subscription_id: Option<String>,
        code: String,
        message: String,
    },
    /// One handler→client frame on a bidi session. Frames are
    /// emitted in handler write order (per-session FIFO). No
    /// cross-direction ordering guarantee with `SendBidi`.
    RecvBidi { session_id: String, frame: Value },
    /// Per-frame bidi diagnostic. Carries `session_id` so it
    /// correlates with an open bidi session.
    ///
    /// Per C-M3a §D5: receiving `ErrorBidi` does NOT close the
    /// session — it is a data-plane diagnostic, not a terminal
    /// signal. Only `TerminalBidi` closes a session. The handler
    /// decides whether to continue after surfacing diagnostics.
    ///
    /// Used both for OpenBidi failures (handler refused, registry
    /// lookup miss) and for in-session per-frame errors. When the
    /// open itself failed, no `TerminalBidi` follows — the failed
    /// open never produced a half-open session per §I3.
    ErrorBidi {
        session_id: String,
        code: String,
        message: String,
    },
}

/// Well-known error codes. Values are stable wire tokens; changing
/// one breaks every Client binding.
pub mod codes {
    /// Inbound frame was not valid JSON or did not match any variant
    /// of `IncomingFrame`. The connection stays open — the client
    /// can retry with a well-formed frame.
    pub const PROTOCOL: &str = "protocol";
    /// Ability name is unknown to this daemon.
    pub const NOT_FOUND: &str = "not_found";
    /// Ability-specific error surfaced by the handler.
    pub const ABILITY_FAILED: &str = "ability_failed";
    /// IPC handshake version negotiation rejected the peer.
    pub const VERSION_INCOMPATIBLE: &str = "version_incompatible";
    /// Server is shutting down; no new work will be accepted.
    pub const SHUTTING_DOWN: &str = "shutting_down";
    /// Daemon accepted the control connection before its dispatcher
    /// finished booting. The caller should subscribe to
    /// `system.watch_boot` or retry after Ready.
    pub const BOOTING: &str = "booting";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invoke_frame_round_trips_through_json() {
        // The wire format is JSON today (proto JSON mapping v2).
        // Serialising and re-parsing must yield an identical struct
        // so the client/server implementations can be tested against
        // fixture strings rather than byte buffers.
        let f = IncomingFrame::Invoke {
            request_id: "r-1".into(),
            ability: "observe.health".into(),
            args: serde_json::json!({}),
            subject: None,
        };
        let s = serde_json::to_string(&f).unwrap();
        let back: IncomingFrame = serde_json::from_str(&s).unwrap();
        match back {
            IncomingFrame::Invoke {
                request_id,
                ability,
                ..
            } => {
                assert_eq!(request_id, "r-1");
                assert_eq!(ability, "observe.health");
            }
            _ => panic!("expected Invoke variant after round-trip"),
        }
    }

    #[test]
    fn subscribe_frame_round_trips_optional_subject() {
        let f = IncomingFrame::Subscribe {
            subscription_id: "sub-42".into(),
            ability: "device.camera.subscribe".into(),
            args: serde_json::json!({"fps": 5}),
            subject: Some("easynet:///r/localhost/resource/cam".into()),
        };
        let s = serde_json::to_string(&f).unwrap();
        let back: IncomingFrame = serde_json::from_str(&s).unwrap();
        match back {
            IncomingFrame::Subscribe {
                subscription_id,
                ability,
                subject,
                ..
            } => {
                assert_eq!(subscription_id, "sub-42");
                assert_eq!(ability, "device.camera.subscribe");
                assert_eq!(
                    subject.as_deref(),
                    Some("easynet:///r/localhost/resource/cam")
                );
            }
            _ => panic!("expected Subscribe variant after round-trip"),
        }
    }

    #[test]
    fn unknown_incoming_variant_fails_with_protocol_level_error() {
        // A client sending a frame with `type: "nope"` must get a
        // parse failure the server can map to `codes::PROTOCOL`,
        // not a silent ignore.
        let raw = r#"{"type":"nope","request_id":"x"}"#;
        let r: Result<IncomingFrame, _> = serde_json::from_str(raw);
        assert!(r.is_err(), "unknown variant must fail to parse");
    }

    #[test]
    fn outgoing_result_frame_carries_request_id_verbatim() {
        // Load-bearing for client correlation: the request_id on
        // Result must match the one sent on Invoke verbatim. A
        // regression that lower-cases ids or strips hyphens would
        // be invisible at compile time; this test pins it.
        let f = OutgoingFrame::Result {
            request_id: "Req-42-XYZ".into(),
            value: serde_json::Value::Null,
            receipt_header: None,
        };
        let s = serde_json::to_string(&f).unwrap();
        assert!(s.contains("\"request_id\":\"Req-42-XYZ\""));
    }

    #[test]
    fn outgoing_result_frame_omits_receipt_header_field_when_none() {
        // Wire-shape contract: when the dispatch path can't resolve
        // the owner Agent (e.g. pre-join state), the Result frame
        // must NOT emit a receipt_header field — older Clients that
        // don't decode the field stay compatible.
        let f = OutgoingFrame::Result {
            request_id: "x".into(),
            value: serde_json::Value::Null,
            receipt_header: None,
        };
        let s = serde_json::to_string(&f).unwrap();
        assert!(
            !s.contains("receipt_header"),
            "absent header must not serialize; got {s}"
        );
    }

    #[test]
    fn open_bidi_frame_round_trips_through_json() {
        // Inbound bidi-open carries the same correlation-id triple
        // the rest of the C-M3a machinery keys on. Pin the wire shape
        // so a future schema generator producing protobuf-JSON sees
        // the snake_case discriminator and field names this enum
        // already commits to.
        let f = IncomingFrame::OpenBidi {
            session_id: "s-1".into(),
            ability: "device.terminal.attach".into(),
            args: serde_json::json!({"node":"01DEV"}),
        };
        let s = serde_json::to_string(&f).unwrap();
        assert!(s.contains("\"type\":\"open_bidi\""), "discriminator: {s}");
        assert!(s.contains("\"session_id\":\"s-1\""));
        let back: IncomingFrame = serde_json::from_str(&s).unwrap();
        match back {
            IncomingFrame::OpenBidi {
                session_id,
                ability,
                ..
            } => {
                assert_eq!(session_id, "s-1");
                assert_eq!(ability, "device.terminal.attach");
            }
            _ => panic!("expected OpenBidi after round-trip"),
        }
    }

    #[test]
    fn send_bidi_and_close_bidi_round_trip_through_json() {
        // Pinned together because both share the session_id key and
        // a regression that swaps "session_id" for any other token
        // would break the receiver's correlation lookup. One test,
        // two frames, fewer redundant fixtures.
        let send = IncomingFrame::SendBidi {
            session_id: "s-2".into(),
            frame: serde_json::json!({"data":"hello"}),
        };
        let send_json = serde_json::to_string(&send).unwrap();
        assert!(send_json.contains("\"type\":\"send_bidi\""));
        let _: IncomingFrame = serde_json::from_str(&send_json).unwrap();

        let close = IncomingFrame::CloseBidi {
            session_id: "s-2".into(),
        };
        let close_json = serde_json::to_string(&close).unwrap();
        assert!(close_json.contains("\"type\":\"close_bidi\""));
        let _: IncomingFrame = serde_json::from_str(&close_json).unwrap();
    }

    #[test]
    fn recv_bidi_and_terminal_bidi_and_error_bidi_carry_session_id_only() {
        // Per design D6, the bidi outbound trio uses `session_id`
        // (not `subscription_id`). A regression that emits the wrong
        // key here means clients correlate against an empty field —
        // visible only at runtime under high load. Pin the field
        // name explicitly.
        let recv = OutgoingFrame::RecvBidi {
            session_id: "s-3".into(),
            frame: serde_json::json!({"k":"v"}),
        };
        let s = serde_json::to_string(&recv).unwrap();
        assert!(s.contains("\"type\":\"recv_bidi\""));
        assert!(s.contains("\"session_id\":\"s-3\""));
        assert!(!s.contains("subscription_id"));

        let term = OutgoingFrame::TerminalBidi {
            session_id: "s-3".into(),
            reason: "done".into(),
        };
        let s = serde_json::to_string(&term).unwrap();
        assert!(s.contains("\"type\":\"terminal_bidi\""));
        assert!(s.contains("\"session_id\":\"s-3\""));
        assert!(!s.contains("subscription_id"));

        let err = OutgoingFrame::ErrorBidi {
            session_id: "s-3".into(),
            code: codes::ABILITY_FAILED.into(),
            message: "boom".into(),
        };
        let s = serde_json::to_string(&err).unwrap();
        assert!(s.contains("\"type\":\"error_bidi\""));
        assert!(s.contains("\"session_id\":\"s-3\""));
        assert!(!s.contains("subscription_id"));
    }

    #[test]
    fn outgoing_result_frame_emits_receipt_header_field_when_present() {
        let header = crate::runtime::hosted_receipt::HostedAgentReceiptHeader::new_selfsigned(
            "easynet:///r/acme/device/01DEV",
        )
        .unwrap();
        let f = OutgoingFrame::Result {
            request_id: "x".into(),
            value: serde_json::Value::Null,
            receipt_header: Some(header),
        };
        let s = serde_json::to_string(&f).unwrap();
        assert!(s.contains("receipt_header"));
        assert!(s.contains("01DEV"));
    }
}
