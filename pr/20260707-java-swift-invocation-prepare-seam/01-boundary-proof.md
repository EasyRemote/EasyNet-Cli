# Boundary Proof

## Layer Ownership

The Java and Swift packages remain language SDK facades. They own idiomatic DTOs,
error mapping, and transport delegation. They do not own Axon canonicalization,
daemon policy, admission, receipt verification, keyring authority, or product
routing.

## Invocation State Machine

The seam exposes four public pre-runtime states:

1. `InvocationBuilder`: mutable construction helper.
2. `InvocationDraft`: immutable complete seven-tuple snapshot.
3. `PreparedInvocation`: immutable canonical signing material, not executable.
4. `SignedInvocation`: immutable submit-ready envelope.

Only `SignedInvocation` can pass to `RuntimeClient.submitSigned`. This prevents a
facade from collapsing canonical material and submit-ready state.

## Canonical Material Delegation

`RuntimeClient.prepare` sends the complete draft JSON and options JSON to the
injected transport. The transport returns `PreparedInvocation` JSON. The Java and
Swift facades validate shape, descriptor binding, request identity, and
submit-readiness, but they do not derive canonical bytes or hashes.

## No Legacy Aliases

The seam uses current fixture fields only:

- `caller_ura`
- `callee_ura`
- `descriptor_ref`
- `subject_ura`
- `nonce_base64`
- `causal_context`
- `args`
- `content_type`
- `metadata`

No legacy input aliases are accepted for new invocation material objects.

## Product Boundary

The implementation introduces no EasyNet-specific or EasyRemote-specific
abstractions. URA strings are carried as opaque runtime identifiers. Product
behavior remains downstream of the SDK.
