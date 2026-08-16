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
