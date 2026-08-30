# Verification

- `cargo check -p easynet --features axon-pb --lib` passed.
- Target proof tests passed: 20/20.
- Target tracker tests passed: 16/16.
- Session tests passed: 23/23.
- Native media tests passed: 2/2.
- New focused tests prove:
  - provider layout is accepted only for the exact pending window-id set;
  - candidate failure restores the active session and committed epochs;
  - native candidate failure emits no `MEDIA_SOURCE_LOST`.
- daemon, CLI, and keyring binaries built successfully with `axon-pb`.
- Runtime restarted with the new daemon and reached
  `FRONTEND_CONNECTED`, online, session admitted.
- Real macOS application browser lifecycle passed twice:
  - `target/e2e/frontend-remoteapp-browser-lifecycle/20260824-014301-36509`
  - `target/e2e/frontend-remoteapp-browser-lifecycle/20260824-014337-38432`
