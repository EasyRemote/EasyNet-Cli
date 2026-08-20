# EasyRemote Mission Events And Profile Transport Conformance

## Objective

Make EasyRemote's Admin/Mission product adapters structurally conform to the
EasyNet-Cli Python SDK profile transport protocols. This keeps EasyRemote as a
consumer facade over SDK-owned profile clients instead of a partial, untyped
adapter that only works for the methods current tests happen to call. Expose
daemon-owned Mission event pages through the SDK Mission adapter and
EasyRemote's `MissionControl` / `MissionRun` handles.

## Boundary

- EasyNet-Cli SDK owns Admin/Mission request DTOs, result projection, retry
  taxonomy, and profile transport protocols.
- EasyRemote owns product-level invocation into its existing client and may
  expose ergonomic agent/mission helpers.
- EasyRemote may expose page-based Mission event access, but event DTO
  validation, cursor semantics, and event projection stay in the SDK Mission
  profile.
- EasyRemote must not redefine SDK retry hints or use product-only method names
  as the profile contract.
- Unsupported profile methods in the EasyRemote product adapter must terminate
  explicitly through typed SDK errors; they must not silently fall back to raw
  carriers or local reimplementation.

## Non-goals

- Do not change `docs/spec/daemon-sdk-requirements-v1.md`.
- Do not implement backend/Hub Admin, session, pairing, or mission-event
  behavior inside EasyRemote.
- Do not add live-tail Mission streaming until the daemon/ABI exposes a bounded
  stream contract; this slice is page-based `events()`.
