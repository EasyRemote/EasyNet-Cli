# API Contract

No public API changes.

## Request tuple

The Axon invocation tuple remains caller, callee, subject, ability, payload, metadata, and call mode.

## Error behavior

When device ownership is not materialized in trust state, ordinary policy resolves the owner as unresolved and returns the existing policy-denied shape instead of silently projecting from local credentials.

## Tenant rule

Tenant ownership must come from canonical trust facts or canonical URA ownership. A tenant-local credentials file cannot assign owner authority for ordinary runtime admission.
