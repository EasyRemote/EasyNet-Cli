# Invocation Target Callee Required

## Goal

Remove the invocation wire helper that resolved a route target from `callee` with `caller` fallback. Canonical invocation routing must use the explicit callee tuple field.

## Non-goals

- Do not change descriptor-bound ability refs.
- Do not change signer custody or key-service behavior in this iteration.
- Do not introduce a compatibility path for caller-only envelopes.

## Acceptance criteria

- Local unary, stream, bidi, and carrier-v1 dispatch all use a callee-only target helper.
- Caller-only envelopes fail before route resolution.
- Convergence gates reject reintroduction of caller fallback target resolution.
