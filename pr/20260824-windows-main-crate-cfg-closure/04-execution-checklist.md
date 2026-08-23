# Execution checklist

- [x] Bound Tokio `UnixStream` and its pumps to Unix.
- [x] Add fail-closed non-Unix executor entry points.
- [x] Bound Agent purge open-handle identity validation to Unix.
- [x] Run mutation gate, host checks, and Windows cross-build.
- [x] Prepare an isolated commit without staging parallel invocation/raw-stream work.
