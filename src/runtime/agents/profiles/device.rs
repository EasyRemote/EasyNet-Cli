//! device profile — RFC-001 §1.
//!
//! An Agent advertising fleet.* + observe.* + admin.* + meta.* +
//! schedule.* + loop.* + discuss.* abilities. Default-on; one per
//! easynet-daemon instance. Represents the local host's
//! operational surface.
//!
//! Per RFC §A4: "device" is an implementation profile, NOT a
//! protocol type. The Agent has no `kind` field on the wire.
//!
//! Owned ability namespaces (per plan §1)
//! --------------------------------------
//!   fleet.*       (wired in agents/skill_ability.rs + session_ability.rs etc.)
//!   observe.*     (wired in agents/ping.rs as observe.health)
//!   schedule.*    (wired in agents/schedule_ability.rs)
//!   loop.*        (wired in agents/loop_ability.rs)
//!   discuss.*     (wired in agents/discuss_ability.rs)
//!   meta.*        (TBD — landed in P3+ when reflexive abilities ship)
//!   admin.*       (TBD — landed in P3+ when admin.{drain,snapshot,...} ship)

/// Standard ability-name prefixes a device-profile Agent may
/// advertise. Used by the daemon's advertise loop (P3) to filter
/// the registry's full ability list down to the device-profile's
/// portion.
pub const DEVICE_PROFILE_ABILITY_PREFIXES: &[&str] = &[
    "fleet.",
    "observe.",
    "schedule.",
    "loop.",
    "discuss.",
    "meta.",
    "admin.",
];

/// Returns true if `ability_name` is owned by the device profile.
pub fn owns(ability_name: &str) -> bool {
    DEVICE_PROFILE_ABILITY_PREFIXES
        .iter()
        .any(|p| ability_name.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owns_recognizes_every_documented_namespace() {
        assert!(owns("fleet.list_abilities"));
        assert!(owns("observe.health"));
        assert!(owns("schedule.add"));
        assert!(owns("loop.create"));
        assert!(owns("discuss.create"));
        assert!(owns("meta.describe"));
        assert!(owns("admin.snapshot"));
    }

    #[test]
    fn owns_rejects_other_profiles() {
        assert!(!owns("consent.subscribe"));
        assert!(!owns("policy.evaluate"));
        assert!(!owns("mcp.bridge.call_tool"));
        assert!(!owns("conversation.send"));
        assert!(!owns("federation.join"));
    }
}
