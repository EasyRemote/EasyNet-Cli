# Node history authority preflight convergence

## Goal

Close the cross-language history-authority seam that lets Node/product callers
submit receipt history requests with a stale or placeholder session subject and
only discover the mismatch after daemon admission.

## Root abstraction problem

Go and Python already model `SessionHistoryOperations` as an authority-bearing
runtime operation: the receipt query call tuple is validated before the receipt
provider is reached, and receipt filters remain post-admission ledger
predicates. Node had tuple-bound invocation metadata validation, but no
equivalent history operation boundary, so product code could assemble
`invocation.history.list` requests from ad hoc metadata and push deterministic
`AUTHORITY_SUBJECT_MISMATCH` failures into daemon admission.

## Boundary proof

- The SDK owns generic runtime request DTOs only: `RuntimeCallContext`,
  `ReceiptFilter`, `ReceiptListRequest`, `ReceiptHistoryPage`, and
  `SessionHistoryOperations`.
- No EasyNet/EasyRemote product concept is introduced.
- `SessionHistoryOperations.list()` validates the complete authority-bearing
  runtime tuple before calling the receipt provider.
- Caller/callee filters may only narrow the authorized tuple.
- Subject filters remain ledger predicates and do not change or repair the
  authority subject.
- All-zero principals and mismatched session subjects fail before provider I/O.

## Verification plan

- Node runtime-core tests for provider preflight and subject filter separation.
- Node syntax check.
- SPEC v2 self-test and main gate.
- Legacy architecture convergence gate.
- Canonical public API regeneration/check if public inventory changes.
- `git diff --check`.
