Invariants:

1. Owner-offline route failures must not be projected as daemon/runtime offline.
2. Caller signer projection remains separate from owner-offline projection.
3. ABI integer stability is preserved; public callers still get a non-zero
   status, while typed bindings read canonical JSON.
4. The projection must cover unary, stream, and bidi daemon status errors because
   all three share `ffi_daemon_error`.
5. Generic `Unavailable` without owner-offline facts remains runtime offline.
