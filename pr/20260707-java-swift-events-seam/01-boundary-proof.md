# Boundary Proof

## Layer Ownership

The Events profile is a daemon SDK profile seam. Java and Swift own typed DTOs,
client lifecycle, error mapping, and transport delegation. The daemon/runtime
owns event production, subscription execution, stream ordering, resume behavior,
and terminal facts.

## Runtime Boundary

`EventClient` methods build or consume daemon profile DTOs over injected
transports. They do not open backend SSE/WebSocket channels, maintain product
subscriber registries, or authorize browser sessions.

## Identity Boundary

Event request carriers preserve complete runtime identity fields:

- `caller_ura`
- `callee_ura`
- `subject_ura`
- `descriptor_version`
- `nonce_base64`
- `causal_context`

Events preserve device, agent, owner, tenant, subject, and invocation references
as current SDK fields. No URI naming is introduced.

## Session Boundary

Session event streams use daemon `session_id`. Product `session_ura` parsing is
explicitly rejected because converting product resources into daemon session ids
would make the language facade own product/session policy.

## State Machine

`EventStream` wraps Runtime Core stream handles. It starts `Live`, moves to
`Terminal` when a terminal event frame is observed, and moves to `Cancelled` or
`Closed` through explicit local operations. It does not fabricate terminal
success.
