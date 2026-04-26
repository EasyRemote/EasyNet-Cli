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
    Invoke {
        request_id: String,
        ability: String,
        #[serde(default)]
        args: Value,
    },
    /// Streaming ability call. Expect zero or more `Frame` envelopes
    /// followed by exactly one `Terminal` envelope.
    Subscribe {
        subscription_id: String,
        ability: String,
        #[serde(default)]
        args: Value,
    },
    /// Early-terminate an in-flight subscription. Server responds
    /// with a `Terminal` envelope on the matching subscription_id.
    Cancel { subscription_id: String },
}

/// One outbound envelope written back to the Client FFI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutgoingFrame {
    /// Single-shot RPC response. Matches an `Invoke` frame's
    /// `request_id`.
    ///
    /// `receipt_header` is the §A12 / §1.3 staging shape carrying
    /// `callee_agent_uri` / `signer_agent_uri` / signing model.
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
    /// Error envelope, used for both Invoke failures and Subscribe
    /// failures. Exactly one of `request_id` / `subscription_id` is
    /// populated depending on which inbound frame caused the error.
    Error {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subscription_id: Option<String>,
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
    fn outgoing_result_frame_emits_receipt_header_field_when_present() {
        let header =
            crate::runtime::hosted_receipt::HostedAgentReceiptHeader::new_selfsigned(
                "easynet:///r/acme/agent/01DEV",
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
