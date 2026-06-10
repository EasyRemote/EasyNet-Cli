// EasyNet CLI — Boot/status control frames
// ========================================
//
// File: src/services/control/frames.rs
// Description: Length-prefixed JSON frame schema for the local
//              `control.sock` boot/status plane.
//
// Boundary
// --------
// This schema is not a product ability transport. Product ability
// calls use daemon `Invocation` over `daemon.sock`; this file only
// models the small control protocol needed before and during daemon
// boot, especially `system.watch_boot`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One inbound boot/status control envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IncomingFrame {
    /// Subscribe to a daemon control stream.
    ///
    /// Today the only accepted `ability` is `system.watch_boot`.
    /// Unknown names receive an `Error` with `code = not_found`.
    Subscribe {
        subscription_id: String,
        ability: String,
        #[serde(default)]
        args: Value,
    },
    /// Early-terminate an in-flight control subscription.
    Cancel { subscription_id: String },
}

/// One outbound boot/status control envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutgoingFrame {
    /// One control stream frame.
    Frame {
        subscription_id: String,
        frame: Value,
    },
    /// Control stream terminal frame.
    Terminal {
        subscription_id: String,
        reason: String,
    },
    /// Control-plane error.
    Error {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subscription_id: Option<String>,
        code: String,
        message: String,
    },
}

/// Well-known control-plane error codes.
pub mod codes {
    /// Inbound frame was not valid JSON or did not match a supported
    /// control envelope. The connection stays open.
    pub const PROTOCOL: &str = "protocol";
    /// Unknown control stream name or subscription id.
    pub const NOT_FOUND: &str = "not_found";
    /// IPC handshake version negotiation rejected the peer.
    pub const VERSION_INCOMPATIBLE: &str = "version_incompatible";
    /// Server is shutting down; no new work will be accepted.
    pub const SHUTTING_DOWN: &str = "shutting_down";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscribe_frame_round_trips_through_json() {
        let f = IncomingFrame::Subscribe {
            subscription_id: "sub-42".into(),
            ability: "system.watch_boot".into(),
            args: serde_json::json!({}),
        };
        let s = serde_json::to_string(&f).unwrap();
        assert!(s.contains("\"type\":\"subscribe\""));
        let back: IncomingFrame = serde_json::from_str(&s).unwrap();
        match back {
            IncomingFrame::Subscribe {
                subscription_id,
                ability,
                ..
            } => {
                assert_eq!(subscription_id, "sub-42");
                assert_eq!(ability, "system.watch_boot");
            }
            _ => panic!("expected Subscribe variant after round-trip"),
        }
    }

    #[test]
    fn cancel_frame_round_trips_through_json() {
        let f = IncomingFrame::Cancel {
            subscription_id: "sub-42".into(),
        };
        let s = serde_json::to_string(&f).unwrap();
        assert!(s.contains("\"type\":\"cancel\""));
        let back: IncomingFrame = serde_json::from_str(&s).unwrap();
        match back {
            IncomingFrame::Cancel { subscription_id } => {
                assert_eq!(subscription_id, "sub-42");
            }
            _ => panic!("expected Cancel variant after round-trip"),
        }
    }

    #[test]
    fn retired_product_incoming_variant_fails_to_parse() {
        let raw = r#"{"type":"invoke","request_id":"x","ability":"observe.health"}"#;
        let r: Result<IncomingFrame, _> = serde_json::from_str(raw);
        assert!(r.is_err(), "retired product frames must not parse");
    }

    #[test]
    fn frame_and_terminal_preserve_subscription_id() {
        let frame = OutgoingFrame::Frame {
            subscription_id: "sub-1".into(),
            frame: serde_json::json!({"type":"ready"}),
        };
        let s = serde_json::to_string(&frame).unwrap();
        assert!(s.contains("\"type\":\"frame\""));
        assert!(s.contains("\"subscription_id\":\"sub-1\""));

        let terminal = OutgoingFrame::Terminal {
            subscription_id: "sub-1".into(),
            reason: "done".into(),
        };
        let s = serde_json::to_string(&terminal).unwrap();
        assert!(s.contains("\"type\":\"terminal\""));
        assert!(s.contains("\"subscription_id\":\"sub-1\""));
    }
}
