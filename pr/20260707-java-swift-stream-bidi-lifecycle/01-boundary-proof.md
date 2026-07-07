# Boundary Proof

## Ownership

The language facade owns local handle state, bounded retained-history behavior,
and typed misuse errors. Provider transports own remote stream/session
registration, owner-scope checks, and daemon-side terminal delivery.

## Invariants

- `close` is idempotent for local stream and bidi handles.
- Bidi `closeSend` closes only the local send side.
- Bidi receive state remains usable after local `closeSend`.
- Sending after local `closeSend` fails with `CANCELLED`.
- Bidi full `close` releases the local session handle and subsequent operations
  fail with `INVALID_HANDLE`.
- No product-specific lifecycle state is introduced.

## Compatibility

Existing public handle APIs remain compatible. The only behavior change is
stricter typed rejection of invalid sends after local send-side closure.
