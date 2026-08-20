Intent: close the C-ABI projection gap where daemon `Unavailable` owner-offline
route failures still look like runtime/daemon downtime.

Observed risk:
- Rust daemon and direct Go/Python providers now distinguish
  `DESCRIPTOR_OWNER_OFFLINE`.
- C-ABI `ffi_daemon_error` still maps every tonic `Unavailable` to
  `ERR_DAEMON_DOWN` with typed last-error `RUNTIME_OFFLINE`.

Architecture target:
- C-ABI integer codes remain stable.
- Typed last-error JSON must carry the canonical runtime code
  `DESCRIPTOR_OWNER_OFFLINE` whenever daemon status detail proves route owner
  liveness failure.
