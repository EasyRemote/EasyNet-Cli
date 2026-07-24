# API Contract

## Request

Allowed top-level fields:

- `ability_ura`: required canonical Ability URA.
- `args`: optional JSON object forwarded to the target ability.

Rejected top-level fields:

- Any non-schema field, including `_caller_ura`, `_request_id`, `_idempotency_key`, `_timeout_ms`, or any future underscore-prefixed field.

## Response

No response contract change. Valid invocations still return the existing result envelope.

## Errors

Invalid top-level fields produce `invalid_args: unknown field ...`.

## Tenant and authority rules

Caller, subject, request, idempotency, and timeout facts are authority-plane/runtime-envelope facts. They must be provided by the canonical invocation builder and verified by daemon/Axon admission, not by this ability parser.
