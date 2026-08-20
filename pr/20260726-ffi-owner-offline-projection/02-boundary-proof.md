Boundary proof:

- `ffi_daemon_error` is the shared C-ABI boundary for daemon `InvokeStatus`,
  `InvokeStreamStatus`, and `InvokeBidiStatus`.
- Therefore owner-offline canonicalization belongs there, not in each carrier.
- The C ABI integer code cannot express all canonical runtime codes without an
  ABI version change; the existing `ErrorProjection` mechanism is the correct
  state-preserving boundary for typed SDKs.
