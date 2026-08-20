# API Contract

## Request

Canonical `federation.join` request fields:

- `membership_ura`
- `realm`
- `public_key_hex`
- optional `principal_enrollment`

## Response

No response shape change. `JoinReceipt` remains the canonical receipt body.

## Errors

Unknown fields, including retired `pairing_secret`, must fail closed as invalid request shape.

## Tenant and identity rules

The request realm, hub callee realm, and membership Device URA realm must match. The provisional caller digest must bind to `public_key_hex`.
