# API Contract

## Request

The public FFI request shape remains unchanged:

- `callee_ura`
- `caller_ura`
- `subject_ura`
- `ability`
- `call_mode`
- `provider`

For `provider: "ability_descriptor"`, `subject_ura` is now mandatory and must equal the realm authority URA for the `callee_ura` realm.

## Response

Successful responses remain descriptor projections with the existing fields and source value `runtime_ability_descriptor_provider`.

## Errors

Invalid catalogue subjects return `InvalidRequest` through the existing descriptor-resolution error projection. They do not trigger route resolution, caller signer lookup, or admission.

## Tenant rule

The authority subject realm must match the callee realm. Cross-realm authority subjects are rejected at the FFI boundary.
