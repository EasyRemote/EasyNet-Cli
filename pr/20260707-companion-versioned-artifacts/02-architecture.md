# Architecture

Layering:
- `DesktopCompanionPlan` carries package identity and version.
- Platform adapters derive installed artifact paths from plan identity, version, and declared executable artifact.
- Launcher render/enable/start uses the versioned installed path.
- Launcher disable/remove first proves the launcher points at the versioned installed path.

Boundary proof:
- This is daemon/plugin lifecycle state, not Axon invocation state.
- SDK/FFI DTOs are unchanged; this is an internal supervisor ownership refinement.
