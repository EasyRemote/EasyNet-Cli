# Verification

## Focused Rust Checks

- `cargo test -q invocation_builder --lib`
- `cargo test -q invocation_handle_cancel_before_completion_is_terminal_monotonic --lib`

## Smoke Self-Tests

- `bash tools/scripts/go-sdk-live-smoke.sh --self-test`
- `bash tools/scripts/python-sdk-live-smoke.sh --self-test`

## Live Daemon Smokes

- `bash tools/scripts/go-sdk-live-smoke.sh`
  - Passed.
  - Observed typed terminal failure: `code=ADMISSION_DENIED stage=runtime`.
  - Observed stream daemon frame.
  - Observed bidi file transfer data and terminal frame.
- `bash tools/scripts/python-sdk-live-smoke.sh`
  - Passed.
  - Observed typed terminal failure: `code=ADMISSION_DENIED stage=runtime`.
  - Observed stream daemon frame.
  - Observed bidi file transfer data and terminal frame.

## Boundary Regression Fixed

The first signed-handle live run exposed C ABI projection defects:

- prepared tuple JSON leaked `timeout_seconds` into the SDK Invocation DTO;
- prepared tuple JSON used a digest-only runtime tuple instead of the SDK-facing draft;
- signing material omitted the descriptor and algorithm fields required by Go/Python DTOs;
- Rust signed-submit result state projected numeric Axon state values.

The implementation now projects the SDK-facing draft JSON at prepared/result boundaries and preserves named terminal states.

## Aggregate Gates

- `bash tools/scripts/check-sdk-cutover-readiness.sh --self-test`
  - Passed.
- `bash tools/scripts/check-sdk-completion-audit.sh`
  - Passed.
  - Re-ran product smokes, Python SDK live smoke, and Go SDK live smoke.
- `git diff --check`
  - Passed.
