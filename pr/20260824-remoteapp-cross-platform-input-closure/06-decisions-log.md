# Decisions log

## 2026-08-24

- Extend the existing plugin platform boundary instead of adding input logic to
  Axon, the Hub, or the frontend.
- Use native OS APIs rather than a cross-platform automation abstraction so
  permission/error/focus behavior stays explicit and auditable.
- Support Linux X11 first; Wayland input requires an xdg-desktop-portal
  RemoteDesktop session and must remain unavailable until that lifecycle is
  integrated with the selected capture/session binding.
- Treat Windows/Linux source and cross-compilation as `baseline_ready` only.
  Capability projection carries `live_e2e_required`; it does not infer product
  readiness from the presence of an OS API implementation.
