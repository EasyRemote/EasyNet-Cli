# Architecture

The device-sponsored RemoteApp SystemAgent owns the public abilities and the
selected Resource subject. The daemon plugin owns session lifecycle and selects
one platform capture provider. Each provider consumes the committed target
binding and emits bounded video/audio samples; the frontend only presents the
resulting WebRTC tracks.

## Provider split

- macOS production capture remains ScreenCaptureKit + VideoToolbox.
- Windows/Linux use an xcap + OpenH264 WebRTC baseline until native accelerated
  providers have live-host evidence. The baseline may be executable without
  being advertised as flagship/production-ready.
- A Window binding resolves one exact `(window_id, owner)` pair.
- An Application binding resolves one exact owner process and a committed
  window-id set. Its software compositor captures only those windows into a
  bounded union surface; it never widens to a monitor.
- The platform observer independently re-enumerates host windows and drives the
  existing rebind/loss state machine when identity, geometry, focus, visibility,
  or an application window set changes.
