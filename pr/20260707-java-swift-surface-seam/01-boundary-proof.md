# Boundary Proof

## Layer Ownership

The Surface profile is a daemon SDK profile seam. Java and Swift own typed DTOs,
client lifecycle, error mapping, and transport delegation. The daemon owns page
ability execution, page publication facts, manifests, health checks, and carrier
construction. Backend products own HTTP rendering, browser authorization, and
SSE/WebSocket/public-route fanout.

## Runtime Boundary

`SurfaceClient` methods accept complete request DTOs and delegate carrier
construction or projection to injected `SurfaceTransport` instances. The
facades validate request/projection shape and do not construct descriptor refs
from string fragments.

## Identity Boundary

Surface request carriers preserve complete runtime identity fields:

- `caller_ura`
- `callee_ura`
- `subject_ura`
- `descriptor_version`
- `nonce_base64`
- `causal_context`

Surface projections preserve daemon-governed `owner_ura`, `surface_ref`, and
`public_ref` fields. No URI naming is introduced.

## Product Boundary

The seam does not expose filesystem transport, backend rendering policy, page
route authorization, or product database row identity. Folders are request data
for daemon page publication; they are not opened or inspected by the SDK.
