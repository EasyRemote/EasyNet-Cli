# Execution Checklist

- [x] Read architecture skills.
- [x] Run codegraph and targeted source search.
- [x] Select active runtime/SDK legacy surface.
- [x] Refactor canonical owner and migrate tests/callers.
- [x] Run focused tests and gates.
- [x] Commit if stable.

## Iteration 2 — Rejected local read issuer split

- [x] Re-read architecture skills.
- [x] Run codegraph and targeted issuer search.
- [x] Select active subject-owner conflation in `LocalRuntimeStateReadIssuer`.
- [x] Prototype explicit Device-owned read issuer.
- [x] Reject the prototype because the SPEC v2 runtime-state read boundary gate requires the selected CLI read paths to enter through `LocalRuntimeStateReadIssuer`.
- [x] Remove the prototype and restore the previous production source state.
- [x] Preserve the finding as an architecture decision instead of committing a gate-divergent implementation.

## Iteration 3 — Node receipt-history governance subject parity

- [x] Re-run codegraph and targeted source search for `invocation.history.list`, `meta.list_abilities`, descriptor resolution, and authority subject mismatch paths.
- [x] Identify Node SDK history admission divergence from Go/Python: Node accepted only user runtime-state subjects, while Go/Python also accept an exact callee runtime-owner subject.
- [x] Split Node history subject admission into explicit user runtime-state and runtime-owner predicates.
- [x] Add Node tests for exact device runtime-owner subject with delegation authority and non-callee runtime-owner rejection.
- [x] Clear local EasyNet state through the product purge path after user authorization.
- [x] Run focused tests and gates.
- [x] Commit if stable.

## Iteration 4 — Swift receipt canonicalizer fail-closed parity

- [x] Re-run codegraph/search for URI terminology, receipt canonicalizer defaults, and cross-language governance subject parity.
- [x] Identify Swift `RuntimeReceipt.canonicalReceiptType` as a fail-open helper returning an empty string for unknown canonical lifecycle states.
- [x] Refactor Swift receipt type binding to throw on unknown canonical lifecycle state.
- [x] Add direct Swift regression coverage for unknown canonical lifecycle state.
- [x] Run focused Swift tests and repository gates.
- [x] Commit if stable.
