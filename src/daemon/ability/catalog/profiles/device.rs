//! Device profile — RFC-001 §1.
//!
//! The daemon-hosted Agent anchored at the local device URA. This is not the
//! physical device object itself. The physical device hosts resources and the
//! daemon; the device-profile Agent advertises the public abilities that
//! operate on those resources under device authority.
//!
//! Default-on; one per `easynet-daemon` instance. It advertises the local
//! host's operational surface: filesystem, process, terminal, session,
//! browser/media/voice, skill package management, observe/admin/meta, and
//! schedule/loop/discuss abilities that run in the daemon.
//!
//! Per RFC §A4: "device" is an implementation profile, NOT a
//! protocol type. The Agent has no `kind` field on the wire.
//!
//! Descriptor projection
//! ---------------------
//! The dispatch registry stores an `OwnerKind` for every ability at
//! registration time. Device profile descriptors are generated from entries
//! whose authority/projection class is exactly `OwnerKind::Device`; this module
//! does not infer ownership from ability name prefixes.

/// Build AbilityDescriptors for every system ability the registry flags as
/// advertised by the device-profile Agent under device authority, with the
/// visibility + scope defaults from RFC plan §18.
///
/// Wire shape — for each name in the registry whose owner is
/// `OwnerKind::Device`:
///   * observe.*  → PUBLIC
///   * everything else (fs/process/terminal/session/device/admin/
///     schedule/loop/discuss/meta/skill/media/etc.) → SCOPED with
///     scope_subjects/scope_agents = Any (P4.1 default).
///     P4.7 narrows the SCOPED axes to the host operator URA on
///     daemon boot.
///
/// `owner_ura` is the device-profile Agent's canonical URA,
/// minted at first `federation.join` and persisted via
/// `local-agents.json`. Caller passes it in so this module stays
/// pure (no daemon-state coupling).
pub fn descriptors_for(
    owner_ura: &str,
) -> Vec<crate::daemon::ability::descriptors::AbilityDescriptor> {
    use crate::daemon::ability::descriptors::{AbilityDescriptor, Visibility};
    use crate::daemon::ability::dispatch::OwnerKind;

    let mut out = Vec::new();
    for meta in
        crate::daemon::ability::catalog::published_system_abilities_for_owner(OwnerKind::Device)
    {
        let visibility = if meta.name.starts_with("observe.") {
            Visibility::Public
        } else {
            Visibility::Scoped
        };
        let public_name = crate::core::ura::owner_local_ability_name(owner_ura, &meta.name);
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
            // Device agent lifecycle is device-profile-owned. These abilities
            // must be advertised by the device projection before RFC-005
            // resolve-before-invoke can start or refresh hosted agents.
            "agent.start",
            "agent.stop",
            "agent.refresh",
            "meta.list_resources",
            // Session timeline state is daemon-local and therefore belongs
            // to the device-profile Agent, not the LLM sub-agent whose run
            // produced a given event.
            "session.list",
            "session.attach",
            // Skill management and skill package file browsing are
            // device-profile-owned because the package tree lives on this
            // host.
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
        use crate::daemon::ability::descriptors::Visibility;
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
