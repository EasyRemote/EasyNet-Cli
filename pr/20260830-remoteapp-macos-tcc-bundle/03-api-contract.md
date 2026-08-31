# API Contract

No public Invocation or HTTP shape changes.

- `remote_desktop.permission_status` returns the app's inner executable path and the
  physical TCC result.
- `remote_desktop.request_permission` invokes the same bundled executable that
  owns active ScreenCaptureKit sessions.
- `remote_desktop.create_session` continues to fail closed with
  `target_permission_missing` until that identity is granted.
- Backend catalog selection reports installed transport capability; only the
  media-host permission state machine reports TCC authorization.
