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
