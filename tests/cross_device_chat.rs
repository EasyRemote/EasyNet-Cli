//! Tier-3 cross-device chat fixtures (PR-1 placeholder)
//!
//! PR-1 owns only the presence / wrapper / session transport
//! substrate. The end-to-end device-A → hub → device-B chat path is
//! explicitly deferred to PR-3, but the checklist requires the
//! fixture file to exist now so the eventual owner has a stable test
//! home to fill in rather than inventing a new path later.

#[test]
#[ignore = "PR-3 owns the real cross-device chat transport path"]
fn cross_device_chat_round_trip_fixture_pending_pr3() {
    // Placeholder only. The real implementation lands once
    // `canonical session dispatch` + chat ability routing are both live.
    // Keeping a concrete ignored test here turns the checklist item
    // into a tracked artefact instead of a TODO in prose.
}
