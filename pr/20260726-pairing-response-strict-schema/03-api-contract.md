# API contract

## Request/response

The Hub pairing validate endpoint response is strict JSON:

- Required identity facts: `node_id`, `credential_token`, `hub_endpoint`,
  `realm`, `username`, `user_id`.
- Product metadata facts may be present only under canonical field names.
- Unknown fields are rejected.

## Error rule

Schema skew is reported as an unreadable pairing response by the existing join
operator-facing error path; the underlying serde cause remains in the error
chain.

## Tenant rule

The canonical product realm field is `realm`. `tenant_id` is retired at pairing
response ingress and must not be accepted as an alias or ignored extension.
