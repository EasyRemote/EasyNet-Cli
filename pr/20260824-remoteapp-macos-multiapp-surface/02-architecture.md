# Architecture

The macOS resource provider aggregates application windows by stable app
identity across displays and records exact window IDs plus union geometry.
It also records front-to-back per-window geometry with a deterministic
`surface_layout_epoch`, distinct from the stable window-set identity epoch.

The plugin-owned ScreenCaptureKit adapter resolves each committed `SCWindow`
to a desktop-independent content filter. A bounded compositor retains only the
latest BGRA frame per surface, reuses unchanged window frames, alpha-composites back-to-front into one
CoreVideo pixel buffer, and submits at most the negotiated FPS to the existing
VideoToolbox encoder. WebRTC, session lifecycle, authority, receipts, and
terminal closure remain unchanged.

The target observer recomputes both proofs from a fresh host snapshot. Window
membership changes advance identity and media epochs; layout-only changes
advance only the media epoch. The pending binding becomes active only after the
complete replacement capture plan starts and revalidates its proof. Pointer
input separately samples the front-to-back native window list immediately
before dispatch and rejects gaps or foreign occlusion.

All native plan sinks pass through one generation router. A replacement plan
can warm up while muted. Rebind pauses delivery, asks the Runtime-owned session
aggregate to commit the pending binding, then activates the new generation or
restores the old one. This prevents both pre-commit leakage and the stale-epoch
case where capture changes after the binding commit is rejected.
