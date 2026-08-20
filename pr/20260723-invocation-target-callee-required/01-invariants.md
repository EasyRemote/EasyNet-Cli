# Invariants

1. Route target identity is the invocation envelope callee, never a synthesized caller fallback.
2. Caller identity remains authority/proof input, not a routing target substitute.
3. Missing callee is an invalid tuple and must fail before namespace or LocalRuntime resolution.
4. All invocation modes share the same callee target extraction semantics.
5. No public wire field or API surface changes; the existing tuple contract is enforced more strictly.
