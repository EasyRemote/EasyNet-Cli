// EasyNet CLI — System Abilities (`system.*` namespace)
// ======================================================
//
// File: src/runtime/system/mod.rs
// Description: Device-level abilities published by `easynet-daemon`.
//              Distinct from agent abilities (which live under
//              `runtime::abilities` and bind to one registered AI
//              agent), system abilities belong to the *node*
//              itself: ping, schedule, session-attach, permission,
//              discuss, loop. Their handlers run inside the daemon,
//              not inside an agent subprocess.
//
// Naming
// ------
// All system abilities are named `system.<feature>[.<verb>]`. Today
// only `system.ping` exists; PR-ATTACH onwards extends the namespace.
//
// Per-feature module layout
// -------------------------
// One file per feature (PR-ATTACH adds `session_ability.rs`,
// PR-PERM adds `permission_ability.rs`, etc.). Each file exports
// (a) the schema/manifest helpers and (b) a registration function
// that mounts the handler on the `LocalAbilityRegistry`.
//
// CI rule (`scripts/check-dispatch-boundary.sh`)
// ----------------------------------------------
// Handler functions in this directory MUST NOT inspect
// `self.node_id` or `target_node` to decide locality. The stage-1
// resolver in `runtime::invocation_target` is the only place that
// makes that decision; handlers consume `InvocationTarget` and act
// on it.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod discuss_ability;
pub mod permission_ability;
pub mod ping;
pub mod schedule_ability;
pub mod session_ability;

use std::sync::Arc;

use crate::runtime::ability_dispatch::LocalAbilityRegistry;
use crate::runtime::execution::discuss::DiscussService;
use crate::runtime::execution::permission::PermissionService;
use crate::runtime::execution::schedule::ScheduleService;
use crate::runtime::execution::session::SessionService;

/// Build a `LocalAbilityRegistry` populated with every v1 system
/// ability handler. Suitable for early-boot smoke tests + the
/// `published_ability_names` helper that the discovery publisher
/// consumes. Tests get fresh empty sub-services; the daemon bin
/// calls `build_registry_with_services` instead with its real
/// Kernel handles.
pub fn build_registry() -> Arc<LocalAbilityRegistry> {
    build_registry_with_services(
        Arc::new(SessionService::new()),
        Arc::new(PermissionService::new()),
        Arc::new(DiscussService::new()),
        Arc::new(ScheduleService::new()),
    )
}

/// Build a `LocalAbilityRegistry` with sub-service handles wired
/// in. The daemon bin calls this with the Kernel's actual handles
/// at boot; tests construct a fresh registry per case.
pub fn build_registry_with_services(
    sessions: Arc<SessionService>,
    perms: Arc<PermissionService>,
    discuss: Arc<DiscussService>,
    schedule: Arc<ScheduleService>,
) -> Arc<LocalAbilityRegistry> {
    let mut reg = LocalAbilityRegistry::new();
    ping::register(&mut reg);
    session_ability::register(&mut reg, sessions);
    permission_ability::register(&mut reg, perms);
    discuss_ability::register(&mut reg, discuss);
    schedule_ability::register(&mut reg, schedule);
    Arc::new(reg)
}

/// Public list of every v1 system-ability *name*. Used by
/// `registry::a2a_labels` to populate the top-level
/// `system_skills[]` field of the node-roster v2 envelope so peers
/// discover what device-level abilities this daemon offers without
/// invoking anything.
///
/// The list is built from the live registry to avoid name drift
/// between the publisher and the dispatcher.
pub fn published_ability_names() -> Vec<String> {
    build_registry().list_abilities()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_registry_is_non_empty_and_includes_ping() {
        // Every v1 daemon publishes at least `system.ping` so a
        // peer wanting to test reachability has a known ability.
        // A regression that emptied this list would silently break
        // discovery + smoke tests.
        let reg = build_registry();
        let names = reg.list_abilities();
        assert!(
            names.iter().any(|n| n == "system.ping"),
            "system.ping must be in the v1 registry; got {names:?}"
        );
    }

    #[test]
    fn published_ability_names_matches_live_registry() {
        // The label-publishing helper and the dispatch registry
        // must agree byte-for-byte. A regression that returned a
        // hard-coded list would let the publisher advertise
        // abilities the dispatcher cannot route.
        let live = build_registry().list_abilities();
        let advertised = published_ability_names();
        assert_eq!(live, advertised);
    }
}
