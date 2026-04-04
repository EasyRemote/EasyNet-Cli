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
pub fn node_state_str(n: &Value) -> Cow<'static, str> {
    if let Some(s) = n.get("state").and_then(|v| v.as_str()) {
        // Map known protocol strings to static &str; allocate only for unknown values.
        return Cow::Borrowed(match s {
            "UNKNOWN" => "UNKNOWN",
            "JOINING" => "JOINING",
            "PROBATION" => "PROBATION",
            "HEALTHY" => "HEALTHY",
            "SUSPECT" => "SUSPECT",
            "QUARANTINED" => "QUARANTINED",
            "DRAINING" => "DRAINING",
            "REMOVED" => "REMOVED",
            _ => return Cow::Owned(s.to_string()),
        });
    }
    if let Some(num) = n.get("state").and_then(Value::as_u64) {
        return Cow::Borrowed(match num {
            1 => "JOINING",
            2 => "PROBATION",
            3 => "HEALTHY",
            4 => "SUSPECT",
            5 => "QUARANTINED",
            6 => "DRAINING",
            7 => "REMOVED",
            _ => "UNKNOWN",
        });
    }
    Cow::Borrowed("UNKNOWN")
}
