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

/// Build AbilityDescriptors for every ability the live registry
/// flags as owned by the device profile, with the visibility +
/// scope defaults from RFC plan §18.
///
/// Wire shape — for each name in the registry that `owns(name)`
/// returns true for:
///   * observe.*  → PUBLIC
///   * everything else (fleet/admin/schedule/loop/discuss/meta) →
///     SCOPED with scope_subjects/scope_agents = Any (P4.1 default).
///     P4.7 narrows the SCOPED axes to the host operator URA on
///     daemon boot.
///
/// `owner_agent_uri` is the device-profile Agent's canonical URA,
/// minted at first `federation.join` and persisted via
/// `local-agents.json`. Caller passes it in so this module stays
/// pure (no daemon-state coupling).
pub fn descriptors_for(
    owner_agent_uri: &str,
) -> Vec<crate::runtime::ability_descriptor::AbilityDescriptor> {
    use crate::runtime::ability_descriptor::{AbilityDescriptor, Visibility};
    let mut out = Vec::new();
    for meta in crate::runtime::agents::published_abilities() {
        if !owns(&meta.name) {
            continue;
        }
        let visibility = if meta.name.starts_with("observe.") {
            Visibility::Public
        } else {
            Visibility::Scoped
        };
        let descriptor = AbilityDescriptor::new(meta.name.clone(), owner_agent_uri, visibility)
            .expect("registry-derived names satisfy descriptor invariants")
            .with_input_schema(meta.input_schema.clone())
            .with_source("kernel:built-in");
        out.push(descriptor);
    }
    out
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

    #[test]
    fn descriptors_for_emit_only_owned_names() {
        let owner = "easynet:///r/acme/agent/01DEV";
        let descriptors = descriptors_for(owner);
        assert!(!descriptors.is_empty(), "device profile must own at least observe.health");
        for d in &descriptors {
            assert!(
                owns(&d.name),
                "device::descriptors_for emitted '{}' which it does not own",
                d.name
            );
            assert_eq!(d.owner_agent_uri, owner);
            assert_eq!(d.source, "kernel:built-in");
        }
    }

    #[test]
    fn descriptors_for_marks_observe_as_public_and_others_scoped() {
        use crate::runtime::ability_descriptor::Visibility;
        let descriptors = descriptors_for("easynet:///r/acme/agent/01DEV");
        for d in descriptors {
            if d.name.starts_with("observe.") {
                assert_eq!(d.visibility, Visibility::Public, "{} must be PUBLIC", d.name);
            } else {
                assert_eq!(d.visibility, Visibility::Scoped, "{} must be SCOPED", d.name);
            }
        }
    }
}
