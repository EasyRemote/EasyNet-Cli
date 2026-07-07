# Boundary Proof

## Layer Ownership

Host Binding is a daemon SDK profile seam. Java and Swift own typed DTOs,
codec/hash request construction, projection validation, client close state, and
endpoint lifecycle state machines. Product hosts own process startup, warm
runtime management, Python/Node/native user-code execution, and socket serving.

## Runtime Boundary

`HostBindingClient` delegates daemon-authored binding and frame projections to
an injected `HostBindingTransport`. The facades may validate request shape and
hash cursor invariants before delegation, but they do not publish abilities,
inspect packages, execute functions, or decide product host warmth.

## Codec Boundary

The shared frame contract is fixed to `host-stream-frame.schema.json`.
Item/error/terminal variants are mutually exclusive. Output hash state follows
`sha256(prev_hash || seq_be || canonical_json(value))` and rejects corrupted
zero-frame state, cursor gaps, and reorders before transport calls.

## Lifecycle Boundary

Lifecycle providers are explicit endpoint hooks. They can check readiness and
cleanup endpoint resources, but cannot execute user code or own host-stream
frame semantics. Cleanup is idempotent, and controller states are decidable:
declared, checking, ready, not_ready, cleaning, cleaned, failed, and closed.
