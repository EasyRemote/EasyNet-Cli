# Invariants

- Runtime-dispatch is daemon-internal executor plumbing; it must not become a
  second public Invocation transport.
- The request shape is latest-only: callers must supply `mode`.
- Missing `mode` is invalid input, not an RPC alias.
- Unknown modes fail loud before dispatch.
- `subject_ura` remains optional envelope context and does not default a
  protocol subject for resource-scoped handlers.
