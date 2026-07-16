# Agent Purge Platform Deletion API Contract

## Public Request Contract

No public API change.

- Request remains `{ "name": string }` or `{ "agent_ura": string }`.
- Admission remains Manage.
- Descriptor hints remain destructive and non-idempotent.

## Public Response Contract

No public response change.

- Successful purge returns the existing tombstone response fields.
- Unsupported platforms return the existing typed internal error before mutation.

## Tenant and Authority Contract

- Device-owned `agent.purge` remains the public catalog entry.
- Platform support is an implementation precondition, not a new tenant routing rule.
- URA semantics remain unchanged.
