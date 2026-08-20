# Verification plan

## Rust checks

- `cargo fmt --all`
- `cargo check -q -p easynet --features remote-desktop,headless-media --lib`
- Focused remote desktop route/provider tests.
- Script boundary tests for lifecycle/input and target binding.

## Static gates

- `bash tools/scripts/check-remoteapp-lifecycle-input-boundary.sh`
- `bash tests/scripts/test_check_remoteapp_lifecycle_input_boundary.sh`
- `bash tools/scripts/check-remoteapp-target-binding-boundary.sh`
- `bash tools/scripts/check-remoteapp-e2e-acceptance-boundary.sh`
- `bash tools/scripts/check-remoteapp-frontend-invocation-boundary.sh`

## Cross-repo checks

- Run the EasyNet frontend remote desktop/media protocol tests after CLI-side checks pass.

## Audit

- `git diff --check`
- Search touched files for forbidden non-URA address terminology.
- `codegraph sync .`
- `codegraph status .`

## Executed verification — 2026-08-16

- `cargo test --features remote-desktop,headless-media --lib remote_desktop -- --nocapture`
  passed all 318 Remote Desktop tests on committed HEAD.
- Every `tools/scripts/check-remoteapp-*.sh` gate passed (7/7), and
  `cargo test --features remote-desktop,headless-media --test script_checks remoteapp -- --nocapture`
  passed all seven mutation-backed script checks.
- Strict library Clippy has no findings in `plugins/remote-desktop` or the
  remote-target resource bootstrap. Repository-wide `-D warnings` remains
  blocked by pre-existing findings in unrelated agent, device-control,
  invocation, routing, and SDK-runner modules.
- The full workspace run reached 5,941 passed, 36 failed, and 7 ignored tests.
  All 36 failures are outside Remote Desktop and reproduce stable existing
  catalog/descriptor/remote-routing baseline drift (for example, the isolated
  ability-layer classification test reports unrelated files/pages/resource
  abilities missing from its classification table).
- EasyNet `Frontend` passed its full 79-file, 642-test suite without retaining
  any additional change in its pre-existing dirty worktree. Its final focused
  Remote Desktop/media-channel run also passed 4 files and 50 tests.
- Go SDK `go test ./...` passed. Python SDK passed 614 tests plus 266 subtests.
  URA naming, product-neutrality, architecture convergence, and canonical
  public API gates passed against the current EasyNet-Axon source revision.
- Fresh Go/Python live conformance reports passed all shared 52-case records
  and the parity matrix with source attestation
  `b046c9584bc4352fbe910bfad0d5ed9cb053b8629b454d9e7258d85cbac771ee`.
- CodeGraph synchronized 1,115 files, 43,884 nodes, and 172,703 edges. Its
  caller trails prove `DirectWebRtcMediaExecution` is the shared boundary used
  by recorder, polling, and native media strategies, and that rebind deadline
  expiry flows from the target state machine through the session/store to the
  observer result consumed by the target monitor.
- Automatic target rebind now expires deterministically at the SPEC 30-second
  bound even when the platform observer returns no observation. Active media
  expiry emits `TARGET_REBIND_FAILED` before `MEDIA_SOURCE_LOST`, clears media
  readiness, disables input, and projects an epoch-fenced endpoint stop.
- Target inventory discovery outages now emit the typed
  `target_inventory_unavailable` watch event. Availability participates in the
  stable inventory hash, and the domain constructor guarantees that an outage
  cannot fabricate `removed_resource_uras`.
- Architecture-owned sources are clean for the forbidden `URI` term after
  excluding vendored dependencies; `git diff --check` and formatting pass.

## Final authoritative host acceptance

The final run used the sibling EasyNet Docker E2E Hub on TLS port `50443` and
the local EasyNet-Cli daemon in `device` mode. All eleven host reports below
record `status=passed` with no errors and collectively satisfy E2E-01 through
E2E-13:

- `target/e2e/remoteapp-picker-freshness-final`: a sentinel window created
  after daemon boot was selected from a live `resource.refresh_remote_targets`
  inventory row with `availability=available` and freshness metadata (E2E-01).
- `target/e2e/remoteapp-permission-final`: user-self permission probing passed;
  display, window, and application resource subjects failed with
  `invalid_argument` (E2E-02).
- `target/e2e/remoteapp-window-final` and
  `target/e2e/remoteapp-application-final`: exact `WindowSurface`/`AppSurface`
  bindings survived a pre-media resource refresh without changing
  `binding_id`; the invocation subject equals the selected resource URA and
  `args.subject` is absent; unrelated sentinels are absent from decoded WebRTC
  frames; scope widening and display fallback are false; production media,
  codec, transport, and client readiness are true (E2E-03, E2E-04, E2E-06,
  E2E-12, E2E-13).
- `target/e2e/remoteapp-stale-window-final`: target close before
  `create_session` returned `target_not_found` plus `refresh_targets`, without
  inserting a session (E2E-05).
- `target/e2e/remoteapp-display-fallback-final`: missing display identity
  returned `display_identity_missing`; no session, media start, first-display
  capture, or decoded frame occurred (E2E-07).
- `target/e2e/remoteapp-window-move-resize-final`: live streaming emitted
  ordered `TARGET_MOVED` revision 2 then `TARGET_RESIZED` revision 3, with the
  input projection consuming the current revision (E2E-08).
- `target/e2e/remoteapp-window-target-loss-final`: target close emitted
  `TARGET_LOST` then `MEDIA_SOURCE_LOST`, suspended target lifecycle, and
  disabled input without misclassifying the loss as a transport failure
  (E2E-09).
- `target/e2e/remoteapp-weak-identity-final`: app/title-only identity returned
  `target_identity_ambiguous`; no session, stream, media, or decoded frame
  started (E2E-10).
- `target/e2e/remoteapp-view-only-window-final` and
  `target/e2e/remoteapp-view-only-application-final`: interactive requests
  without proven target-scoped dispatch were downgraded to `view_only`, and
  pointer/key input is rejected as `input_scope_unsupported` (E2E-11).

Final regression evidence:

- RemoteApp Rust suite: 319 passed / 0 failed.
- RemoteApp static boundaries: 7 passed / 0 failed.
- Mutation-backed RemoteApp script checks: 7 passed / 0 failed.
- EasyNet Frontend selected-subject/protocol/media/input suite: 4 files and 50
  tests passed; the sibling worktree's pre-existing changes were not modified.
- CodeGraph: 1,115 files, 43,895 nodes, and 172,765 edges. The final
  `TargetTrackingEmission` trail has one target-state owner and one session
  event-log projection boundary.
- Fresh Go/Python live conformance: each language emitted 52 records (51
  passed, 1 explicitly unsupported), and the parity matrix passed against Axon
  attestation
  `working_tree:4ea88100962fff6d827d828a8c44ef77eb2b812d0fb724d9c38bf839379cd7f8`.
  Both reports share CLI source attestation
  `9180d4ed0116b6119b2907d662e97c58108c606911f6f623470474b75bf0352e`.
- `cargo fmt --all -- --check`, architecture convergence, SDK product
  neutrality, SDK URA naming, and the 16-test multi-authority conformance suite
  passed. Strict library Clippy reports 25 pre-existing findings in unrelated
  agent/device-control/invocation modules and no finding in any RemoteApp or
  conformance path changed by this implementation.
