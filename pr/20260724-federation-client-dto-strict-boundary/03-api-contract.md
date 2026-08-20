# API Contract

## Request/response shape

- `PrincipalEnrollmentProof` accepts only canonical proof reference fields.
- `AdvertiseAgentReceipt` accepts only canonical advertise acknowledgement fields.
- `ResolvedAgent` accepts only canonical agent row fields.
- `ResolveReceipt` accepts only canonical resolve receipt fields.

## Errors

- Unknown fields fail deserialization and surface as receipt parsing errors.
- No compatibility remapping is performed.

## Tenant and authority rules

- Parsed receipt facts must not create or override tenant, owner, caller, or signer identity through unknown fields.
- Authority and signer state remain owned by canonical runtime admission paths.
