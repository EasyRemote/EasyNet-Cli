# API Contract

Internal contract:
- `DesktopCompanionSessionProbe::probe(platform)` returns `Available` or `Unsupported { reason }`.
- For Linux, availability is determined by `DISPLAY` or `WAYLAND_DISPLAY`.
- For non-Linux platforms, load planning returns `Available` and leaves detailed runtime checks to platform supervisors.

Wire behavior:
- `PluginLoadStatus::CompanionUnsupportedSession` renders as `companion_session_unsupported`.
- The status is non-fatal and package-only; companion packages still do not register ability runtime handlers.
