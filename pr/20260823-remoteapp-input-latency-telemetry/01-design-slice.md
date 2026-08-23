# RemoteApp input latency telemetry slice

Date: 2026-08-23

## Product requirement

RemoteApp pointer/keyboard control must be low-latency in real execution, and
the daemon must emit evidence that can be checked by product E2E artifacts.
Frontend timestamps alone are not sufficient because the host execution path is
the authority for input admission and OS injection.

## Boundary

- The RemoteApp plugin owns high-frequency input data-channel execution.
- The session aggregate still owns lifecycle, target readiness, and transport
  epoch admission.
- Axon receipts remain the control-plane evidence for session abilities; raw
  pointer/key frame latency is plugin event telemetry attached to the active
  RemoteApp session.

## Implemented slice

- Record daemon host receive time for parsed pointer/key frames.
- Emit `host_applied_at_ms` and `latency_ms` on `INPUT_FRAME_APPLIED` events
  when the client supplied `client_sent_at_ms`.
- Preserve host-side timing on coalesced `INPUT_FRAME_REJECTED` diagnostics so
  rejected input storms remain bounded but still auditable.
- Do not fabricate latency when client clock appears ahead of host time.

## Non-claims

This does not prove successful OS input injection, focus correctness, or
cross-device latency. It supplies the missing daemon-side telemetry that the
live input injection E2E must consume.
