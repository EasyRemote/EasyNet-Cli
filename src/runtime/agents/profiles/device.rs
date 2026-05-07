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
//!   observe.*     (wired in agents/ping.rs as device.observe.health)
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
    // device.* — joint-plan unified-path replacement for the
    // self-arm of device.fleet.describe_node. Per the URA `device` role
    // canonicalisation, ability names follow the noun-verb shape:
    // `device.describe` describes "this device". Cross-device
    // routing is the caller's job (forward_invoke against the
    // target device URA), so the ability namespace lives on the
    // device profile.
    "device.",
    // AXIOM Tier 2.5 Baseline Locomotion Profile members. Every
    // host-embodied agent claiming `baseline-locomotion-v1`
    // exposes these via the device profile, so device.meta.list_abilities
    // / federation.advertise must surface them. Pre-fix the
    // prefix table predated these and they were silently absent
    // from the descriptor catalogue even though the dispatcher
    // routed them — surfaced in a real-user audit when
    // device.meta.list_abilities returned 31 entries while the live
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
    // RFC-005 v3.2 A1–A8 — physical-channel media abilities
    // owned by the device profile (the host holds the hardware).
    // The bare `"device."` prefix already captures
    // `device.voice.*`; the explicit `mic.` / `camera.` /
    // `screen.` / `speaker.` legacy prefixes stay as a
    // defense-in-depth fallback for any stale catalogue entry
    // emitted under the pre-M2 bare-namespace shape.
    //
    // Pre-fix the LLM profile claimed `device.voice.*` on the
    // theory that voice signaling was an LLM-owned surface. The
    // handlers actually run on the device daemon
    // (`OwnerKind::Device` in the registry); claiming them in
    // the LLM profile caused the catalogue's
    // `descriptors_for(agent_uri)` to stamp every voice verb
    // with the agent URA, so `easynet ability list` grouped
    // them under AGENT and the KIND column read `agent`.
    // Ownership here reflects "where does the handler run" per
    // the truth-table spec, not "which surface category the
    // verb semantically belongs to" — the call signaling state,
    // SDP / ICE candidates, and audio capture all live on the
    // host, so device-owned is the honest classification.
    "mic.",
    "camera.",
    "screen.",
    "speaker.",
];

/// Returns true if `ability_name` is owned by the device profile.
///
/// **Profile precedence**: M2 of the system-namespace migration
/// moved every system verb under `device.*`. The device profile's
/// `"device."` prefix matches them all, but consent / policy /
/// mcp / llm sub-profiles claim certain `device.<sub>.*` shapes
/// FIRST. Without this exclusion the device profile would
/// shadow the more-specific sub-profiles, breaking
/// `descriptors_for_<profile>(uri)` ownership.
///
/// Rule: a name is device-owned iff it matches a device prefix
/// AND is NOT owned by any non-device profile (consent, policy,
/// mcp, llm). The check goes through the actual sub-profile
/// `owns()` functions so any future prefix change in a sub-
/// profile flows through automatically.
pub fn owns(ability_name: &str) -> bool {
    if super::consent::owns(ability_name)
        || super::policy::owns(ability_name)
        || super::mcp::owns(ability_name)
        || super::llm::owns(ability_name)
    {
        return false;
    }
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
        // M2 of system-namespace migration: catalogue entries are
        // canonical (`device.observe.*`, `device.fleet.*`, …); the
        // visibility split that previously branched on the legacy
        // `observe.` prefix follows suit.
        let visibility = if meta.name.starts_with("device.observe.") {
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
        assert!(owns("device.fleet.list_abilities"));
        assert!(owns("device.observe.health"));
        assert!(owns("device.schedule.add"));
        assert!(owns("device.loop.create"));
        assert!(owns("device.discuss.create"));
        assert!(owns("device.meta.describe"));
        assert!(owns("device.admin.snapshot"));
        // Tier 2.5 Baseline Locomotion Profile members.
        assert!(owns("device.fs.read"));
        assert!(owns("device.fs.write"));
        assert!(owns("device.fs.list"));
        assert!(owns("device.fs.edit"));
        assert!(owns("device.process.exec"));
        assert!(owns("device.shell.run"));
        assert!(owns("device.http.request"));
        // A2A edge adapters (bridge = inbound, client = outbound).
        assert!(owns("device.a2a.bridge.list_skills"));
        assert!(owns("device.a2a.bridge.send_task"));
        assert!(owns("device.a2a.client.send_task"));
        // Joint-plan device.* unified-path namespace.
        assert!(owns("device.describe"));
        // RFC-005 v3.2 voice signaling — moved here from the
        // LLM profile because the handlers run on the device
        // daemon (`OwnerKind::Device`). Catalogue entries get
        // stamped with the device URA and the Frontend Agents
        // page renders them under the DEVICE / SYSTEM section.
        assert!(owns("device.voice.create_call"));
        assert!(owns("device.voice.subscribe"));
        assert!(owns("device.voice.transcribe"));
    }

    #[test]
    fn baseline_locomotion_seven_are_all_owned_by_device_profile() {
        // Pin the AXIOM Tier 2.5 Baseline Locomotion contract.
        // Every member of the seven-ability profile MUST be
        // claimed by device::owns; otherwise device.meta.list_abilities
        // and federation.advertise silently drop them on
        // non-joined hosts.
        for name in [
            "device.fs.read",
            "device.fs.write",
            "device.fs.list",
            "device.fs.edit",
            "device.process.exec",
            "device.shell.run",
            "device.http.request",
            // PTY family — inhabits fleet.* prefix already, but
            // let's pin them here so a future renamer trips this
            // test instead of silently breaking the catalog. The
            // unary I/O trio (input/read/resize) lives alongside
            // attach (bidi) so the backend's PTYDriver — which
            // talks unary RPC — sees a fully-served wire surface.
            "device.fleet.pty_session_create",
            "device.fleet.pty_session_close",
            "device.fleet.pty_session_attach",
            "device.fleet.pty_session_input",
            "device.fleet.pty_session_read",
            "device.fleet.pty_session_resize",
        ] {
            assert!(
                owns(name),
                "{name} is a Baseline Locomotion ability and MUST be \
                 owned by the device profile; otherwise device.meta.list_abilities \
                 / federation.advertise will not surface it"
            );
        }
    }

    #[test]
    fn owns_rejects_other_profiles() {
        assert!(!owns("device.consent.subscribe"));
        assert!(!owns("device.policy.evaluate"));
        assert!(!owns("device.mcp.bridge.call_tool"));
        assert!(!owns("conversation.send"));
        assert!(!owns("federation.join"));
    }

    #[test]
    fn descriptors_for_emit_only_owned_names() {
        let owner = "easynet:///r/acme/device/01DEV";
        let descriptors = descriptors_for(owner);
        assert!(
            !descriptors.is_empty(),
            "device profile must own at least device.observe.health"
        );
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
        let descriptors = descriptors_for("easynet:///r/acme/device/01DEV");
        for d in descriptors {
            // Post-M2 of system-namespace migration: every device
            // verb is partitioned under `device.*`. observe.* is
            // PUBLIC (the federation-tier liveness surface);
            // everything else stays SCOPED.
            if d.name.starts_with("device.observe.") {
                assert_eq!(
                    d.visibility,
                    Visibility::Public,
                    "{} must be PUBLIC",
                    d.name
                );
            } else {
                assert_eq!(
                    d.visibility,
                    Visibility::Scoped,
                    "{} must be SCOPED",
                    d.name
                );
            }
        }
    }
}
