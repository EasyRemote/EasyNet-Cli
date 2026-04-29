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
    // AXIOM Tier 2.5 Baseline Locomotion Profile members. Every
    // host-embodied agent claiming `baseline-locomotion-v1`
    // exposes these via the device profile, so meta.list_abilities
    // / federation.advertise must surface them. Pre-fix the
    // prefix table predated these and they were silently absent
    // from the descriptor catalogue even though the dispatcher
    // routed them — surfaced in a real-user audit when
    // meta.list_abilities returned 31 entries while the live
    // registry had 49.
    "fs.",
    "process.",
    "shell.",
    "http.",
    // a2a.* edge adapters: the bridge is an inbound A2A server,
    // the client is an outbound A2A caller. These are stateless
    // host-level adapters (no separate hosted agent owns them,
    // unlike mcp.* which is gated on the hosted mcp agent).
    // Putting them under device matches their actual deployment
    // model: every host that participates in A2A has them.
    "a2a.",
    // RFC-005 v3.2 A1–A5, A8 — physical-channel media abilities
    // owned by device-profile (the host holds the hardware).
    // voice.* / voice.transcribe are llm-profile-owned and live
    // in `profiles/llm.rs`; the prefix list here intentionally
    // omits "voice." for that reason.
    "mic.",
    "camera.",
    "screen.",
    "speaker.",
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
            .with_source("kernel:built-in")
            .with_description(meta.description);
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
        // Tier 2.5 Baseline Locomotion Profile members.
        assert!(owns("fs.read"));
        assert!(owns("fs.write"));
        assert!(owns("fs.list"));
        assert!(owns("fs.edit"));
        assert!(owns("process.exec"));
        assert!(owns("shell.run"));
        assert!(owns("http.request"));
        // A2A edge adapters (bridge = inbound, client = outbound).
        assert!(owns("a2a.bridge.list_skills"));
        assert!(owns("a2a.bridge.send_task"));
        assert!(owns("a2a.client.send_task"));
    }

    #[test]
    fn baseline_locomotion_seven_are_all_owned_by_device_profile() {
        // Pin the AXIOM Tier 2.5 Baseline Locomotion contract.
        // Every member of the seven-ability profile MUST be
        // claimed by device::owns; otherwise meta.list_abilities
        // and federation.advertise silently drop them on
        // non-joined hosts.
        for name in [
            "fs.read", "fs.write", "fs.list", "fs.edit",
            "process.exec", "shell.run", "http.request",
            // PTY family — inhabits fleet.* prefix already, but
            // let's pin them here so a future renamer trips this
            // test instead of silently breaking the catalog. The
            // unary I/O trio (input/read/resize) lives alongside
            // attach (bidi) so the backend's PTYDriver — which
            // talks unary RPC — sees a fully-served wire surface.
            "fleet.pty_session_create",
            "fleet.pty_session_close",
            "fleet.pty_session_attach",
            "fleet.pty_session_input",
            "fleet.pty_session_read",
            "fleet.pty_session_resize",
        ] {
            assert!(
                owns(name),
                "{name} is a Baseline Locomotion ability and MUST be \
                 owned by the device profile; otherwise meta.list_abilities \
                 / federation.advertise will not surface it"
            );
        }
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
