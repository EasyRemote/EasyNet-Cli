# Node Events Seam Intent

Add a Node/TypeScript Events profile seam that matches the shared daemon SDK
runtime model in `docs/spec/daemon-sdk-requirements-v1.md`.

## Scope

- Expose typed Node Events carriers for directory, device, invocation, and
  session event operations.
- Delegate Invocation carrier construction and live stream opening to an
  injected Events transport.
- Project daemon-authored event frames, dropped-event reports, terminal frames,
  and device history pages into stable SDK DTOs.
- Declare Node only for Events conformance cases whose behavior is covered by
  Node tests.

## Out Of Scope

- No daemon provider, C ABI provider, or local daemon socket transport.
- No SDK-local event bus, fan-out loop, post-filtering, or backend SSE policy.
- No product session URA parser; session streams require daemon `session_id`.
