# Boundary Script Fixture Sandbox Plan

## Objective

Keep SDK scaffold and browser/voice boundary gates executable in isolated test
sandboxes without weakening production source checks.

## Boundary

- Do not modify `docs/spec/daemon-sdk-requirements-v1.md`.
- Do not relax production browser or voice surface checks.
- Only exclude test fixture scaffolding from rules that target production
  process-global handlers.

## Invariants

1. `tests/scripts/test_check_sdk_scaffold.sh` must copy every script that
   `check-sdk-scaffold.sh` requires.
2. Browser session service boundary checks must still reject process-global
   handlers in production code.
3. Voice call Axon contract checks must still reject retired compatibility
   fields in production code.
4. Test-only `OnceLock` fixtures must not make the boundary gates unusable.

## Verification

- `bash tests/scripts/test_check_sdk_scaffold.sh`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `bash tools/scripts/check-browser-session-service-boundary.sh`
- `bash tools/scripts/check-voice-call-axon-contract.sh`
- `git diff --check`
