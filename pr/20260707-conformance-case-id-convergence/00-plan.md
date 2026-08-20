# Conformance Case ID Convergence Plan

## Goal

Converge shared conformance case identity with the latest SDK SPEC so adapter
reports, shared case files, and language tests use the normative case ids
directly.

## Scope

- Rename `invocation/handle_terminal_monotonicity` to
  `invocation/terminal_monotonicity`.
- Split the combined `stream_bidi/lifecycle_state` case into the SPEC-named
  `stream/order_terminal` and `bidi/close_send_not_cancel` cases.
- Update Go/Python shared-case tests, action-adapter reports, parity references,
  route-family evidence, and scaffold guards.

## Non-Goals

- No legacy alias case ids.
- No change to Runtime Core lifecycle semantics.
- No product-specific stream, bidi, route, or UI behavior.
- No new protocol primitive beyond existing stream/bidi lifecycle state.

## Verification

- `bash tools/scripts/check-sdk-conformance-reports.sh`
- `TMPDIR=/tmp bash tools/scripts/check-sdk-scaffold.sh`
- `bash tools/scripts/check-sdk-completion-audit.sh`
- `git diff --check`
