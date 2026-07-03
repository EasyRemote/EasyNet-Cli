## Status

Delivered in this slice:

- Windows local control-plane IPC now uses named pipes.
- FFI unary invoke and subscribe/cancel paths compile against the
  named-pipe transport.
- Local daemon Invocation gRPC now compiles against named pipes on
  Windows, and `easynet runtime start` no longer hard-bails on the
  platform.
- `easynet` + `easynet-daemon` cross-build for
  `x86_64-pc-windows-gnu` succeeds.
- Installer packaging for the Windows zip target succeeds through
  `EasyNet/scripts/deploy-installer.sh`.

Deliberately NOT claimed as done in this slice:

- `runtime-dispatch` / `runtime_local_tools` on Windows. The daemon
  side can still log a non-fatal unsupported error there, and
  EasyNet-Axon's `ipc://` runtime-local-tool client remains UDS-only.
- `easynet-keyring` / self-identity IPC on Windows. Device-mode
  `<self>.session` can still boot from the fallback daemon-identity
  path, but the named-pipe keyring transport is not part of this
  slice.
