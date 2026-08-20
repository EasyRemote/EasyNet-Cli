# Runtime history subject gate convergence

## Goal

Close the selected-route daemon ingress gap where `invocation.history.*` could be forwarded with any Resource URA subject. The canonical runtime model requires a user-owned `runtime-state/read` subject for receipt-history reads.

## Architecture decision

- `RuntimeStateReadSubject` is the single subject value object for runtime-state read projections.
- Selected-route daemon ingress must use the same value object as SDK and CLI helpers.
- Receipt-history reads remain unary-only, and direct remote/local dispatch must reject non-canonical subjects before presence forwarding or LocalRuntime admission.

## Verification

- Focused Rust tests for selected-route history admission.
- Canonical runtime convergence gate.
- Architecture convergence gate.
- Rust formatting.
- Codegraph sync/status.
