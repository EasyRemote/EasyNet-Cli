## Planned checks

- `cargo test --lib daemon::plugins::manifest`
- `cargo test --lib daemon::plugins::package`
- `cargo test --lib daemon::plugins::install::tests`
- `cargo test --lib daemon::plugins::host_api::tests`
- `cargo test --lib daemon::plugins::index::tests`
- `cargo test --lib daemon::plugins::provider_registry::tests`
- `cargo fmt --check`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`

## Results

- `cargo test --lib daemon::plugins::manifest`: passed, 16 tests.
- `cargo test --lib daemon::plugins::package`: passed, 9 tests.
- `cargo test --lib daemon::plugins::install::tests`: passed, 16 tests.
- `cargo test --lib daemon::plugins::host_api::tests`: passed, 8 tests.
- `cargo test --lib daemon::plugins::index::tests`: passed, 7 tests.
- `cargo test --lib daemon::plugins::provider_registry::tests`: passed, 4 tests.
- `cargo fmt --check`: passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`: passed.
- `tools/scripts/check-architecture-convergence.sh`: passed.
- `git diff --check`: passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`: passed.

Note: full `cargo test --lib daemon::plugins` was intentionally not used as the
final acceptance signal for this narrow parser change because the current clean
local runtime has no provisioned device credentials, which trips unrelated
remote-desktop attach tests. The affected parser/package/install/registration
paths are covered by the focused checks above.
