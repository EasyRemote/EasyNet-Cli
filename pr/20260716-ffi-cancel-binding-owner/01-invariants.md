# Invariants

- Shutdown must cancel only invocation resources owned by the closing client
  session generation.
- Cross-generation handle reuse must not cancel another session's resources.
- Stream and bidi cancellation remain idempotent for unknown IDs.
- Public FFI function names and return behavior are unchanged.
- No fallback path from raw handle to cleanup ownership remains in production
  invocation cleanup.
