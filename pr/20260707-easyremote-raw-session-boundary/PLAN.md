# EasyRemote raw session boundary guard

## Goal

Close one SPEC cutover gap by making the EasyRemote boundary audit reject raw
daemon socket/session and runtime subprocess wrappers in product code.

## Invariants

- EasyRemote product code may retain decorators, Python schema extraction, warm
  host process ergonomics, and pipeline DSL semantics.
- EasyRemote product code must not own daemon socket paths, direct daemon
  sessions, gRPC-over-UDS channels, or runtime process bootstrap.
- The SDK facade remains the only allowed boundary for daemon lifecycle,
  Runtime Core transport, and profile execution from EasyRemote.
- This slice adds static enforcement only; it does not introduce compatibility
  aliases or legacy input names.

## Planned edits

- Extend `sdk/python/easynet_sdk/consumer_boundary.py` with raw daemon session
  and runtime subprocess detectors.
- Extend `tools/scripts/check-easyremote-sdk-boundary.sh --self-test` with a
  failing fixture that proves those detectors fire.
- Update EasyRemote conformance cases to declare the newly enforced forbidden
  markers.

## Verification

- `bash tools/scripts/check-easyremote-sdk-boundary.sh --self-test`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `bash tools/scripts/check-sdk-conformance-reports.sh`
- `bash tools/scripts/check-sdk-completion-audit.sh`
- `git diff --check`
