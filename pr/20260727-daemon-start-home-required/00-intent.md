## Goal

Remove the daemon start lifecycle fallback that resolves a missing runtime home
directory to the current working directory.

## Non-goals

- Do not change public daemon start/stop/status APIs.
- Do not change daemon endpoint naming.
- Do not introduce an alternate state directory model.

## Acceptance criteria

- Daemon start path construction requires an explicit child `HOME` override or
  the process `HOME`.
- Blank child `HOME` overrides are rejected before path materialization.
- Missing process `HOME` fails with a typed daemon error instead of using
  `./.easynet`.
- Existing explicit-home SDK/FFI launch behavior remains unchanged.
