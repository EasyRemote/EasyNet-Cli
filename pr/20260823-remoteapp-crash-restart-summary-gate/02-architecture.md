# Architecture

Layering:

1. Crash/restart verifier validates raw runner evidence and emits a compact product summary.
2. Product-completion gate consumes verifier reports and validates only aggregate-required facts.
3. Runtime/daemon implementation remains outside this change.

Boundary decision:

- Deep event ordering and crash semantics remain in `tools/scripts/remoteapp-crash-restart-recovery-e2e.sh`.
- Aggregate sufficiency lives in `tools/scripts/remoteapp-product-completion-e2e.sh`.
- No plugin/runtime execution behavior is moved into test glue.
