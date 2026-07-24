# API Contract

## Request

No request change.

## Response

Canonical heartbeat receipt fields:

- `membership_status`
- `realm_directory_size`
- optional `header`
- optional `rejected_nodes`
- mandatory `hub_abilities_diff`

Retired aliases:

- `status`
- `permanent`

## Errors

Retired aliases must parse as unknown fields.

## Tenant and identity rules

No change. Heartbeat caller/callee/subject binding remains enforced by invocation admission.
