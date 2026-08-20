# Section 27 Conformance Coverage Plan

## Goal

Make SPEC section 27 minimum conformance coverage machine-checkable instead of
implicitly relying on whichever shared cases currently exist.

## Scope

- Add a language-neutral coverage manifest for every SPEC section 27 minimum
  conformance case.
- Add a validator that proves each SPEC case maps to existing shared
  conformance cases.
- Wire the validator into SDK cutover readiness and self-test mode.
- Add exact shared cases for distinct runtime invariants that were only covered
  indirectly.

## Non-Goals

- No product-specific SDK naming.
- No EasyRemote or backend compatibility alias.
- No weakening of section 27 case names.

## Verification

- `bash tools/scripts/check-sdk-section27-coverage.sh`
- `bash tools/scripts/check-sdk-section27-coverage.sh --self-test`
- `bash tools/scripts/check-sdk-cutover-readiness.sh --self-test`
- `bash tools/scripts/check-sdk-completion-audit.sh`
- `git diff --check`
