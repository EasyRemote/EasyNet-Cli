## Intent

Close the Node receipt-history public ingress gap where `RuntimeCallContext` could omit `nonce_base64` or `causal_context` before history authority validation.

## Boundary invariant

- Receipt-history reads are runtime governance invocations and must preserve the complete caller-controlled tuple at the SDK boundary.
- Python and Go already validate complete runtime call context before receipt-history authority checks; Node must not diverge.
- Missing `nonce_base64` or `causal_context` must fail before provider dispatch and before authority-specific interpretation.

## Decision

Route Node `validateSessionHistoryRuntimeCall` through the same complete context validator used by runtime ability calls, then keep the existing history-specific subject and authority checks.

## Verification target

- Node runtime core tests for receipt-history preflight.
- SDK public API/conformance gate.
- Canonical runtime convergence v2 and architecture gates.
