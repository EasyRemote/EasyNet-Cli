# Verification

Planned verification:

- focused target-observer and input policy/unit tests;
- RemoteApp Rust module suite;
- lifecycle/input architecture gate;
- product closure audit gate;
- frontend focused tests if public projections change.

Unit tests prove fail-closed policy and execution boundaries. Product readiness
still requires `remoteapp-input-injection-e2e.sh` with an independent OS effect
observer on a real host.

The evidence verifier now derives `display_global` versus `target_local` from
the selected target kind. Target-local artifacts must include a fresh
per-input guard proof timestamped between host receipt and OS application.

Completed local verification:

- `cargo check --features axon-pb`
- `cargo test --features axon-pb --lib remote_desktop -- --nocapture`
  (`386 passed`)
- input-injection evidence verifier self-test
- view-only input safety verifier self-test
- lifecycle/input architecture boundary gate
- E2E acceptance boundary gate
- product closure audit gate
- JSON parse and `git diff --check`

No live E2E-14 host artifact was produced in this change, so the readiness
matrix remains `partial`.
