Implementation plan:

1. Add owner-offline detail classifier in `src/ffi/invocation/mod.rs`.
2. Add projected-error recorder for `DESCRIPTOR_OWNER_OFFLINE`.
3. Route daemon status errors through that projection before generic status
   code mapping.
4. Add tests proving owner-offline status records canonical typed JSON while
   generic unavailable remains runtime offline.
5. Strengthen SPEC v2 gate for the FFI projection boundary.
6. Run targeted tests, fmt, architecture/SPEC gates, and diff check.
