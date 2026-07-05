# Python EasyRemote Mission Run File

## Goal

Implement the EasyRemote Mission profile `run_file` dispatch path through the SDK-owned mission bridge while leaving invocation-builder carrier methods unsupported until they can delegate to daemon SDK carrier construction.

## Boundary Proof

- EasyRemote bridge remains a product dispatcher adapter over SDK Mission DTOs.
- `run_file` dispatches the daemon `mission.run` system ability with a file path payload and reuses the existing mission status projection.
- Carrier-builder methods remain out of scope because they require daemon SDK carrier construction, not EasyRemote product dispatch.
- No Mission planner, EAL parser, retry policy, or child Invocation semantics are introduced in the EasyRemote bridge.

## Invariants

- `run_file` requires a non-empty path.
- Optional labels are preserved.
- Dispatcher output must satisfy existing Mission status DTO validation.
- Existing `run_eal`, `track`, `cancel`, and `events` behavior remains unchanged.
- No retired address terminology is introduced in touched files.

## Verification

- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_easyremote_profiles.py`.
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_direct_runtime.py`.
- `go test -count=1 ./...` in `sdk/go`.
- `go test -count=1 -tags easynet_cabi ./...` in `sdk/go`.
- `cargo fmt --check`.
- `bash tools/scripts/check-sdk-scaffold.sh`.
- `git diff --check`.
- Retired address terminology scan over touched files.
