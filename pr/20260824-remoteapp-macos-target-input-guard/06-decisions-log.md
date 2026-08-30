# Decisions log

## 2026-08-24

- Reuse the target observer's host snapshot model instead of duplicating
  CoreGraphics window enumeration inside the input module.
- Treat only the first regular visible window of the frontmost process as the
  focused window; all same-process windows are not equivalent.
- Revalidate before every target-local event. Correct target confinement takes
  priority over optimizing host enumeration; measured latency must decide any
  later bounded-cache design.
- Keep display-global behavior unchanged and keep non-macOS target-local input
  unsupported.
- Clamp mapped pointer coordinates to the selected target bounds and set the
  CoreGraphics scroll event location explicitly; focus validation alone cannot
  confine a scroll delivered at an unrelated global cursor position.
- Inject repeated key-down frames instead of reporting them applied without an
  OS event; client sequence handling remains the replay boundary.
- Rebind the effective input policy to the session aggregate's current
  committed target binding on every frame. A data channel created before an
  application window-set rebind must not validate against the creation-era
  binding.
