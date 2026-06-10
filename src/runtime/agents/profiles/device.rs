//! device profile — RFC-001 §1.
//!
//! An Agent advertising device-hosted operational abilities plus observe,
//! admin, meta,
//! schedule.* + loop.* + discuss.* abilities. Default-on; one per
//! easynet-daemon instance. Represents the local host's
//! operational surface.
//!
//! Per RFC §A4: "device" is an implementation profile, NOT a
//! protocol type. The Agent has no `kind` field on the wire.
//!
//! Descriptor ownership
//! --------------------
//! The dispatch registry stores an `OwnerKind` for every ability at
//! registration time. Device profile descriptors are generated from entries
//! whose owner is exactly `OwnerKind::Device`; this module does not infer
//! ownership from ability name prefixes.

/// Build AbilityDescriptors for every system ability the registry flags as
/// owned by the device profile, with the visibility + scope defaults from RFC
/// plan §18.
///
/// Wire shape — for each name in the registry whose owner is
/// `OwnerKind::Device`:
///   * observe.*  → PUBLIC
///   * everything else (device/admin/schedule/loop/discuss/meta) →
///     SCOPED with scope_subjects/scope_agents = Any (P4.1 default).
///     P4.7 narrows the SCOPED axes to the host operator URA on
///     daemon boot.
///
/// `owner_ura` is the device-profile Agent's canonical URA,
/// minted at first `federation.join` and persisted via
/// `local-agents.json`. Caller passes it in so this module stays
/// pure (no daemon-state coupling).
pub fn descriptors_for(
    owner_ura: &str,
) -> Vec<crate::runtime::ability_descriptor::AbilityDescriptor> {
    use crate::runtime::ability_descriptor::{AbilityDescriptor, Visibility};
    use crate::runtime::ability_dispatch::OwnerKind;

    let mut out = Vec::new();
    for meta in crate::runtime::agents::published_system_abilities_for_owner(OwnerKind::Device) {
        let visibility = if meta.name.starts_with("observe.") {
            Visibility::Public
        } else {
            Visibility::Scoped
        };
        let public_name = crate::ura::owner_local_ability_name(owner_ura, &meta.name);
        let descriptor = AbilityDescriptor::new(public_name, owner_ura, visibility)
            .expect("registry-derived device names satisfy descriptor invariants")
            .with_input_schema(meta.input_schema.clone())
            .with_hints(meta.hints.clone())
            .with_source("kernel:built-in")
            .with_description(meta.description.as_str());
        out.push(descriptor);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_locomotion_abilities_are_all_described_by_device_profile() {
        // Pin the AXIOM Tier 2.5 Baseline Locomotion contract.
        // Every member of the profile MUST be emitted from registry
        // OwnerKind::Device metadata; otherwise meta.list_abilities and
        // federation.advertise silently drop it.
        let descriptors = descriptors_for("easynet:///r/acme/device/01DEV");
        let names = descriptors
            .iter()
            .map(|d| d.name.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        for name in [
            "fs.read",
            "fs.write",
            "fs.stat",
            "fs.list",
            "fs.edit",
            "process.exec",
            "shell.run",
            "http.request",
            // PTY family — pin the full terminal surface here so a
            // future renamer trips this
            // test instead of silently breaking the catalog. The
            // unary I/O trio (input/read/resize) lives alongside
            // attach (bidi) so the backend's PTYDriver — which
            // talks unary RPC — sees a fully-served wire surface.
            "terminal.create",
            "terminal.list",
            "terminal.close",
            "terminal.attach",
            "terminal.input",
            "terminal.read",
            "terminal.resize",
            // Device agent lifecycle is also device-owned. These abilities
            // must be advertised by the device projection before RFC-005
            // resolve-before-invoke can start or refresh hosted agents.
            "agent.start",
            "agent.stop",
            "agent.refresh",
            "meta.list_resources",
            // Skill management and skill package file browsing are
            // device-owned because the package tree lives on this host.
            "skill.install",
            "skill.remove",
            "skill.upgrade",
            "skill.publish",
            "skill.unpublish",
            "skill.list",
            "skill.tree",
            "skill.read_file",
            "skill.write_file",
        ] {
            assert!(
                names.contains(name),
                "{name} must be emitted from registry OwnerKind::Device metadata; got {names:?}"
            );
        }
    }

    #[test]
    fn descriptors_for_does_not_steal_sub_profile_abilities() {
        let descriptors = descriptors_for("easynet:///r/acme/device/01DEV");
        let names = descriptors
            .iter()
            .map(|d| d.name.as_str())
            .collect::<std::collections::BTreeSet<_>>();

        for name in [
            "consent.subscribe",
            "policy.evaluate",
            "mcp.bridge.call_tool",
            "conversation.send",
            "federation.join",
        ] {
            assert!(
                !names.contains(name),
                "{name} is not OwnerKind::Device and must not be described by the device profile"
            );
        }
    }

    #[test]
    fn descriptors_for_emit_only_owned_names() {
        let owner = "easynet:///r/acme/device/01DEV";
        let descriptors = descriptors_for(owner);
        assert!(
            !descriptors.is_empty(),
            "device profile must own at least observe.health"
        );
        for d in &descriptors {
            assert_eq!(d.owner_ura, owner);
            assert_eq!(d.source, "kernel:built-in");
        }
        let fs = descriptors
            .iter()
            .find(|d| d.name == "fs.read")
            .expect("fs.read must publish under the device owner-local name");
        assert_eq!(
            fs.canonical_ability_ura().as_deref(),
            Some("easynet:///r/acme/ability/device.01DEV.fs.read")
        );
    }

    #[test]
    fn descriptors_for_marks_observe_as_public_and_others_scoped() {
        use crate::runtime::ability_descriptor::Visibility;
        let descriptors = descriptors_for("easynet:///r/acme/device/01DEV");
        for d in descriptors {
            if d.name.starts_with("observe.") {
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
