# RemoteApp input focus epoch gate

## Intent

Close one input execution seam: pointer and keyboard frames must bind not only
to a geometry revision but also to the selected target focus epoch before OS
injection. Without a daemon-owned focus epoch, a browser/client can replay an
input frame after focus churn and still satisfy stale geometry checks.

## Boundary

- Runtime/plugin layer: `plugins/remote-desktop`.
- No Axon protocol changes.
- No frontend changes in this slice; the daemon input contract becomes stricter
  and exposes the required focus epoch in the effective input policy.
- Do not touch unrelated invocation dispatcher files currently dirty in the
  checkout.

## Invariants

1. `TargetTrackerSnapshot` owns the current `target_focus_epoch`.
2. Focus epoch is positive from session creation and increments on real focus
   transitions.
3. Effective input policy projects the expected focus epoch to clients.
4. Pointer/key frames must carry the expected focus epoch when the daemon has
   one; missing or stale epochs are rejected before platform input injection.
5. Applied input event payloads include the accepted focus epoch so external OS
   effect probes can bind observed effects to the same focus state.
6. View-only policy rejection remains fail-closed and does not require clients
   to satisfy focus/geometry gates first.

## Verification

- PASS:
  `rustfmt --edition 2021 --check plugins/remote-desktop/src/input.rs plugins/remote-desktop/src/target_tracking.rs`
- PASS:
  `cargo test -p easynet --features axon-pb input_rejects_stale_target_focus_epoch_before_os_injection -- --nocapture`
- PASS:
  `cargo test -p easynet --features axon-pb pointer_input_rejects_stale_target_geometry_revision_before_os_injection -- --nocapture`
- PASS:
  `cargo test -p easynet --features axon-pb pointer_policy_consumes_latest_target_tracker_snapshot -- --nocapture`
- PASS:
  `cargo test -p easynet --features axon-pb tracker_disables_input_when_target_loses_focus -- --nocapture`
- PASS:
  `cargo test -p easynet --features axon-pb target_tracking -- --nocapture`
- PASS:
  `bash tools/scripts/check-remoteapp-product-closure-audit.sh`

## Non-scope observation

`cargo test -p easynet --features axon-pb input -- --nocapture` is not a clean
verification target in the current dirty checkout because the substring filter
also selects unrelated invocation routing tests. One such test failed under the
existing dirty invocation dispatcher files:
`resolve_query_json_ignores_retired_camel_case_input_aliases`. The RemoteApp
input tests selected by this slice passed after the focus epoch gate fix.

## Decision

The daemon now treats `target_focus_epoch` as execution-state evidence owned by
the target tracker. Effective input policy exposes the current expected epoch;
pointer/key frames may carry it; and the input execution boundary rejects
missing or stale focus epochs before platform injection when an expected epoch
exists. Pointer frames still report stale geometry before stale focus so clients
receive the most specific target-local correction first.
