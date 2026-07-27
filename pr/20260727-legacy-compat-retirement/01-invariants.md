# Invariants

- Public invocation ingress must expose caller, callee, ability, subject, nonce, causal context, and args before dispatch.
- Daemon ability routes must enter descriptor-bound LocalRuntime dispatch for admission, proof facts, and terminal receipts.
- SDK packages must remain product-neutral; EasyNet/EasyRemote names belong downstream.
- Legacy input handling may exist only as fail-closed validation or versioned edge translation explicitly required by a public API contract.
- Tests must not rely on stale local state; reset paths may delete incompatible state rather than repair it through compatibility fallbacks.
