# Runtime Dispatch Latest Mode Boundary Plan

## Goal

Remove the daemon-internal runtime-dispatch legacy mode fallback so request shape uses the latest explicit `mode` field only.

## Scope

- Require runtime-dispatch requests to carry `mode`.
- Reject missing mode as bad input instead of defaulting to RPC.
- Extend the latest-input boundary guard so future compatibility aliases or fallback mode paths fail in CI.

## Non-Scope

- No change to public daemon Invocation transport semantics.
- No change to stream or RPC response frame shape after a valid explicit mode is supplied.
- No compatibility alias for stale runtime-dispatch callers.
