# Runtime Core Payload Validation

## Goal

Align Go and Python Runtime Core raw-payload validation by rejecting malformed `arguments_base64` before an `InvocationDraft` can be built or decoded.

## Non-goals

- Do not change the public `args` / `arguments_base64` schema.
- Do not add legacy payload aliases.
- Do not move canonical payload hashing into product facades.

## Acceptance Criteria

- Go rejects malformed `arguments_base64` with an SDK invalid-argument error.
- Python rejects malformed `arguments_base64` with an SDK invalid-argument error.
- Existing JSON payload behavior remains unchanged.
- Full Go and Python SDK tests continue to pass.
