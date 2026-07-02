// EasyNet CLI — Services Layer
// =============================
//
// File: src/services/mod.rs
// Description: Long-running daemon service state that has not yet
//              been promoted into the Clean Final daemon source
//              layout. The local control plane already lives under
//              `daemon/control` and daemon Invocation already lives
//              under `daemon/invocation`; this module now holds the
//              remaining migration-stage service stores.
//
// Layering rule
// -------------
// `services/` sits below user-facing `cli`/`ffi` surfaces and reaches the
// runtime through hard trait/type boundaries. The exact allowlist
// is per-subtree because the remaining service families play
// different roles.
//
// `engineering/scripts/check-kernel-boundary.sh` is the CI gate. Adding a new
// permitted import requires updating both the allowlist there AND
// the rationale comment next to it — the script's own header
// documents the convention.
//
// Clean-final direction
// ---------------------
// This module is a migration-stage holding area, not the final
// architecture. New daemon-owned control-plane code goes under
// `daemon/control`; daemon Invocation has moved under
// `daemon/invocation`; quota, persistence, and resource
// state should keep moving toward the semantic directories named in
// `docs/spec/project-structure-v1.md`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

// Admission observability contract (post Phase-5a)
// -------------------------------------------------
// `services/receipt_store.rs` (the in-memory `SharedReceiptStore`
// ring buffer) was deleted in Phase 5a. The current contract:
//
//   * Successful admission of a daemon Invocation
//     emits NO standalone audit artefact. The call proceeds.
//     When (and only when) the invocation reaches a terminal
//     state, the `LedgerSink` writes one `InvocationLedger` row
//     (redb-backed, on-disk).
//
//   * Rejected admission emits an `op_event!`
//     (`component = daemon_invocation`, `kind = admission_rejected_*`)
//     to stderr and surfaces the typed `tonic::Status` to the
//     caller. Nothing is persisted server-side.
//
//   * Forward-invoke (cross-hub) outcomes are observable via the
//     two channels above plus `kind = forward_invoke_*` op_events
//     for transport-level miss/fail diagnostics.
//
// Known gap (DEC-011, scheduled Week-5+): an invocation that
// passes admission but never reaches a terminal state (handler
// hang, daemon crash mid-call) leaves NO record at all. Closing
// this gap with an admission-time ledger sentinel
// (`state = Admitted`) is tracked separately from the Phase 5a
// deletion. Until that lands, the operator log is the audit
// source for non-terminal calls.

/// Service-discovery health metadata for manifest abilities that
/// front an external service (`[health]` probe + optional `[boot]`
/// self-heal). Advisory only — the invoke path never consults it.
pub mod ability_health;
pub mod clipboard_tracker;

/// Owner projection read model updated by `federation.advertise_abilities`.
/// Read by `federation.resolve` when the caller sets `include_abilities =
/// true` so backend pages see bounded RFC-005 ability summaries, not raw
/// implementation descriptors.
#[cfg(feature = "axon-pb")]
pub mod ability_catalog_store;

/// Hosted-agent directory rows published through
/// `federation.advertise_agent`. PresenceRegistry owns transport
/// liveness for device URIs; this store maps hosted agent URIs back
/// to their host device URI so `federation.resolve` can surface
/// `/agent/<user>.<agent>` rows while deriving online/offline from
/// the host's live `session.open`.
pub mod advertised_agent_store;

/// AXON-RFC-001 v4.1.7 hub-broadcast contract — caches hub-owned
/// ability descriptors that the realm hub pushes via
/// `federation.join` (full snapshot) + `federation.heartbeat`
/// (incremental diff). `meta.list_abilities scope=realm` merges
/// this cache with the device-local registry so users see both
/// device-owned and hub-owned abilities through one query.
pub mod hub_published_ability_store;

// `axon_bridge` moved to `crate::runtime::axon_bridge` per the
// 2026-05-29 industrial-textbook review: its imports go almost
// entirely to `crate::runtime::*` (`ability_dispatch`,
// `system_abilities`, `invocation_target`); it carries no `services/`-layer
// concern of its own. Keeping a re-export here would only
// reintroduce the false hierarchy. Search by name (`axon_bridge`)
// or by symbol — every public type is unchanged.

#[cfg(all(test, feature = "axon-pb"))]
mod tests {
    #[test]
    fn quota_meters_user_abilities_but_exempts_control_plane() {
        let meters = crate::daemon::invocation::quota_meter::quota_meters_function;

        assert!(meters("observe.health"));
        assert!(meters("agent.todo.run"));

        assert!(!meters("federation.heartbeat"));
        assert!(!meters("federation.forward_invoke"));
        assert!(!meters("identity.register_pubkey"));
        assert!(!meters("identity.register_pubkey"));
        assert!(!meters("session.open"));
    }
}
