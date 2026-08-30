# Invariants

- Application capture includes only the committed application window IDs.
- Gaps between application windows contain deterministic black pixels, never display pixels.
- Negative virtual-desktop coordinates are valid and must compose without overflow.
- Windows/Linux process-scoped application capture may span displays.
- macOS ScreenCaptureKit application capture remains display-scoped until a real `MultiAppSurface` media source exists.
- Capability metadata must not equate single-display application capture with whole-application multi-display support.
