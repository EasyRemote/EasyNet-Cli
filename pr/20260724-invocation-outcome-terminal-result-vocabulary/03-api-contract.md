# API contract

Public methods:

- `InvocationHandle::await_result`
- `InvocationHandle::result`
- `InvocationOutcome::result`
- `InvocationOutcome::into_result`
- `InvocationOutcome::stages`
- `InvocationOutcome::into_parts`

Behavior:

- No signature, JSON, receipt, error, or state behavior changes.
- Documentation and gates align API semantics with canonical runtime ownership.
