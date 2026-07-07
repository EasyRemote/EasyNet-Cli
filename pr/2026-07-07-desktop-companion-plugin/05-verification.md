# Verification

## Completed Checks

- `cargo fmt --all --check`: passed.
- `git diff --check`: passed.
- `cargo test companion --lib`: passed, 56 tests.
- `cargo test plugin_host --lib`: passed, 34 tests.
- `go test ./...` in `sdk/go`: passed.
- `PYTHONPATH=sdk/python python3 -m pytest sdk/python/tests -q`: passed, 528 tests.
- `mvn test` in `sdk/java`: build passed.
- `swift test --filter RuntimeCoreSeamTests/testCompanionProfileProjectsStateMachineAndLifecycleActions` in `sdk/swift`: passed.
- Search for `URI|uri|Uri` in touched companion/runtime surfaces: no matches.

## Required Coverage

- Valid companion manifest with no abilities.
- Rejection of companion manifests without `[companion]`.
- Rejection of non-companion manifests with `[companion]`.
- Rejection of companion executable artifacts outside hashed roots.
- Package hash changes when `dist/` artifacts change.
- Supervisor install failure rolls back package state.
- Post-Ready start failure remains non-fatal and visible in status.
- Runtime stop skips `keep_running` companions.
- SDK DTO parsing accepts list/status/action shapes.
