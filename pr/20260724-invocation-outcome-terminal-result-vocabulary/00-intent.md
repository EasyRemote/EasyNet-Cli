# Intent

Goal: remove source-compatibility vocabulary from the Rust runtime invocation outcome API and describe `InvocationResult` as the canonical terminal-result projection owned by `InvocationOutcome`.

Non-goals:

- Do not remove or rename public `await_result`, `result`, or `into_result` methods.
- Do not change unary invocation wire shape, receipt verification, or terminal-state derivation.
- Do not add compatibility wrappers or aliases.

Acceptance criteria:

- `InvocationOutcome` documentation describes canonical terminal result and receipt checkpoint ownership.
- No production Rust dispatch client API comment describes `InvocationResult` as a source-compatible DTO.
- Convergence gate rejects reintroduced source-compatible result vocabulary.
