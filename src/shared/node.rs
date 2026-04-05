// EasyNet CLI — Node State Utilities
// ===================================
//
// File: src/shared/node.rs
// Description: Shared node state interpretation for consistent behavior across CLI commands.
//
// Extracted from devices.rs and status.rs to eliminate divergent is_online() implementations.
// Aligned with axon/v1/types.proto NodeState enum.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::borrow::Cow;

use serde_json::Value;

/// Determine whether a node is considered "online" (actively participating in the federation).
///
/// A node is online if:
/// - It has an explicit `online: true` boolean field, OR
/// - Its state is HEALTHY, JOINING, or PROBATION (states that indicate active participation).
pub fn is_online(n: &Value) -> bool {
    if let Some(b) = n.get("online").and_then(Value::as_bool) {
        return b;
    }
    let s = node_state_str(n);
    matches!(&*s, "HEALTHY" | "JOINING" | "PROBATION")
}

/// Map a node's state field to a display string.
/// Handles both string states and numeric protobuf enum values.
///
/// Aligned with axon/v1/types.proto `NodeState` enum:
///   0=UNSPECIFIED, 1=JOINING, 2=PROBATION, 3=HEALTHY, 4=SUSPECT,
///   5=QUARANTINED, 6=DRAINING, 7=REMOVED
/// Known protocol states — used to return `&'static str` without allocation.
const KNOWN_STATES: &[&str] = &[
    "UNSPECIFIED", "JOINING", "PROBATION", "HEALTHY",
    "SUSPECT", "QUARANTINED", "DRAINING", "REMOVED",
];

/// Map an OS identifier to a user-friendly display name.
/// Handles common OS names from `std::env::consts::OS` and `uname`.
/// Case-insensitive matching covers mixed-case inputs from different platforms.
pub fn friendly_os(os: &str) -> &str {
    if os.is_empty() {
        return "";
    }
    if os.eq_ignore_ascii_case("darwin") || os.eq_ignore_ascii_case("macos") {
        "macOS"
    } else if os.eq_ignore_ascii_case("linux") {
        "Linux"
    } else if os.eq_ignore_ascii_case("windows") {
        "Windows"
    } else if os.eq_ignore_ascii_case("android") {
        "Android"
    } else if os.eq_ignore_ascii_case("ios") {
        "iOS"
    } else {
        os
    }
}

pub fn node_state_str(n: &Value) -> Cow<'_, str> {
    let Some(state) = n.get("state") else {
        return Cow::Borrowed("UNKNOWN");
    };
    if let Some(s) = state.as_str() {
        // Return a static reference for known protocol states to avoid allocation.
        // For unknown states, borrow from the Value (no allocation needed).
        return match KNOWN_STATES.iter().find(|&&k| k == s) {
            Some(known) => Cow::Borrowed(*known),
            None => Cow::Borrowed(s),
        };
    }
    if let Some(num) = state.as_u64() {
        // Protobuf numeric enum mapping (axon/v1/types.proto NodeState).
        let idx = usize::try_from(num).unwrap_or(usize::MAX);
        return Cow::Borrowed(
            KNOWN_STATES.get(idx).copied().unwrap_or("UNKNOWN")
        );
    }
    Cow::Borrowed("UNKNOWN")
}
