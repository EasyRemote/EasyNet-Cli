# Device RPC Admission Descriptor

## Concrete use case

The transport admission test must prove that a signed Device caller reaches the
strict RPC verification and replay path. It must not construct an RPC
descriptor for `session.open`, which is a Bidi-only runtime-admin carrier.

## Owner boundary

`session.open` remains owned by the runtime-admin Bidi contract. `session.list`
is the canonical Device-owned RPC representative for this admission test.

## Public compatibility

No route or descriptor is added. The test moves to an existing public RPC
contract and preserves the signed-device policy and nonce-replay assertions.
