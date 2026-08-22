# RemoteApp input backpressure

RemoteApp pointer/keyboard input must be bounded and observable before it can be
called product-grade. The browser currently sends input frames directly to the
WebRTC data channel after policy checks. That preserves authority semantics, but
it does not bound client-side channel backlog or give daemon-side events a
stable client frame sequence for diagnosing latency and drops.

This change closes that concrete seam:

- Frontend input send fails closed when the RTC data-channel backlog exceeds a
  small RemoteApp-specific bound.
- Frontend input frames include a monotonic `client_sequence` and existing
  `sent_at_ms` telemetry.
- The remote-desktop plugin accepts, validates, and projects `client_sequence`
  into applied/rejected input events.

This is not a replacement for full cross-device latency E2E. It is one required
control-plane/data-channel invariant for making that E2E meaningful.
