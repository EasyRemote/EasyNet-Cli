# API Contract

Internal contract:

- `callee_ura_from_envelope(envelope, label)` returns the validated callee URA.
- Missing envelope or missing/empty/invalid callee returns `INVALID_ARGUMENT`.

Public behavior:

- Complete canonical invocations continue to route normally.
- Caller-only envelopes are rejected as incomplete tuples rather than routed to the caller.
