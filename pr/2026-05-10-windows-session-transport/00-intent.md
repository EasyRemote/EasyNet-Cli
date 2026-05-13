# Intent

Goal: make Windows builds not only package, but run the local transport paths required for session lifecycle and session attach flows.

Scope:
- control-plane IPC transport
- local daemon gRPC transport used by CLI helpers
- keyring/self-identity local IPC
- verification focused on session-related flows

Non-goal for this slice:
- full parity of every Unix-only helper if they are not on the session path
