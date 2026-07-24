# Intent

## Goal

Remove the legacy sidecar compatibility seam from the daemon `<agent>.invoke` argument parser so product callers cannot inject `_caller_ura`, `_request_id`, `_idempotency_key`, `_timeout_ms`, or future underscore-prefixed metadata through the business argument object.

## Non-goals

- Do not change the public CLI invoke arguments.
- Do not add a compatibility fallback for legacy IPC payloads.
- Do not move caller identity, request identity, timeout, or idempotency semantics into product-specific code.

## Acceptance criteria

- `<agent>.invoke` accepts only the canonical public input schema: `ability_ura` and optional `args`.
- Any underscore-prefixed top-level field is rejected as an unknown field.
- Sidecar metadata is not parsed from business args and cannot affect audit rows from this handler.
- SPEC v2 gate rejects reintroduction of underscore sidecar compatibility tests or parser logic.
