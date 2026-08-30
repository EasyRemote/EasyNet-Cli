# Architecture

The WebRTC input channel and session aggregate remain platform-neutral. After
consent, sequence, geometry, focus epoch, and fresh target validation, the
plugin dispatches one typed pointer/key frame to a device-local platform
backend.

- macOS: CoreGraphics/Accessibility, unchanged.
- Windows: User32 `SendInput`, with virtual-desktop absolute coordinates and
  bounded wheel/button/key events.
- Linux X11: dynamically loaded X11 + XTest connection guarded by one mutex;
  no build-time dependency on distribution X11 development packages.
- Linux with `WAYLAND_DISPLAY` present: unavailable until the portal
  RemoteDesktop session proves the selected native Wayland/XWayland Resource;
  no synthetic fallback or unproven XTest success claim.

Capabilities distinguish implementation availability from current-host
runtime availability and from live product certification.
