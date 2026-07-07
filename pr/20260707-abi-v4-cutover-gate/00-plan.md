# ABI v4 Cutover Gate Plan

## Goal

Ensure SDK cutover readiness proves the current C ABI v4 header/export/spec
contract.

## Scope

- Add `check-ffi-abi-v4-header.sh` to the SDK cutover readiness gate chain.
- Include its self-test in cutover readiness self-test mode.
- Update the daemon SDK SPEC conformance gate wording from stale ABI v3 to ABI
  v4.

## Non-Goals

- No ABI symbol changes.
- No legacy ABI v3 compatibility path.
- No product-specific C ABI surface.

## Verification

- `bash tools/scripts/check-ffi-abi-v4-header.sh`
- `bash tools/scripts/check-sdk-cutover-readiness.sh --self-test`
- `bash tools/scripts/check-sdk-completion-audit.sh`
- `git diff --check`
