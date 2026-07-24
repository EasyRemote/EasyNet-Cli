# Intent

## Goal

Remove remaining permissive federation client receipt DTO parsing that can admit legacy or product-shaped fields after the canonical runtime convergence cutover.

## Non-goals

- Do not introduce compatibility aliases for older federation receipt shapes.
- Do not change public CLI or SDK-facing behavior except to fail closed on malformed non-canonical federation receipts.
- Do not add product-specific EasyNet or EasyRemote abstractions to the SDK/runtime boundary.

## Acceptance criteria

- Federation client DTOs used for enrollment, advertise, and resolve receipts reject unknown top-level fields.
- Tests prove retired/unknown receipt fields are rejected instead of silently ignored.
- Existing canonical receipt parsing behavior remains intact.
- The change is committed independently as an architecture-boundary refactor.
