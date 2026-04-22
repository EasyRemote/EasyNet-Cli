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
    "UNSPECIFIED",
    "JOINING",
    "PROBATION",
    "HEALTHY",
    "SUSPECT",
    "QUARANTINED",
    "DRAINING",
    "REMOVED",
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

/// Runtime label for a node reached through federation.
///
/// When a node is served through the federation (the local runtime does
/// not own it), the runtime's `list_nodes` handler stamps the originating
/// runtime into the node's `labels` map:
///
///   - `axon.federation.runtime_label` — human-readable name (preferred)
///   - `axon.federation.runtime_id`    — stable id (fallback)
///
/// Locally-owned nodes carry neither key and return `None`.
///
/// Empty-string values are treated as absent; upstream sometimes emits
/// them for partial records, and a blank "via" suffix in the UI would
/// be worse than no hint at all.
pub fn federation_label(n: &Value) -> Option<String> {
    let labels = n.get("labels")?.as_object()?;
    for key in [
        "axon.federation.runtime_label",
        "axon.federation.runtime_id",
    ] {
        if let Some(s) = labels.get(key).and_then(Value::as_str) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
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
        return Cow::Borrowed(KNOWN_STATES.get(idx).copied().unwrap_or("UNKNOWN"));
    }
    Cow::Borrowed("UNKNOWN")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn federation_label_prefers_human_readable_label() {
        let n = json!({
            "labels": {
                "axon.federation.runtime_label": "alpha",
                "axon.federation.runtime_id": "runtime-alpha",
            }
        });
        assert_eq!(federation_label(&n), Some("alpha".to_string()));
    }

    #[test]
    fn federation_label_falls_back_to_runtime_id() {
        let n = json!({"labels": {"axon.federation.runtime_id": "runtime-beta"}});
        assert_eq!(federation_label(&n), Some("runtime-beta".to_string()));
    }

    #[test]
    fn federation_label_returns_none_for_local_node() {
        let n = json!({"labels": {}});
        assert_eq!(federation_label(&n), None);
    }

    #[test]
    fn federation_label_returns_none_when_labels_field_missing() {
        let n = json!({"node_id": "local"});
        assert_eq!(federation_label(&n), None);
    }

    #[test]
    fn federation_label_treats_empty_string_as_absent() {
        let n = json!({
            "labels": {
                "axon.federation.runtime_label": "",
                "axon.federation.runtime_id": "",
            }
        });
        assert_eq!(federation_label(&n), None);
    }
}
