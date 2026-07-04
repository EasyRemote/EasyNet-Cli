# Mission Event Page Conformance

## Objective

Move `mission.events` page projection from an implementation-only seam into
shared SDK conformance. The SDK must prove that Mission event pages are typed,
cursor-bearing daemon projections, and EasyRemote/Pipeline can consume them
without owning raw `mission.events` carriers or event DTO validation.

## Boundary

- EasyNet-Cli SDK owns `MissionEventListRequest`, `MissionEventPage`, cursor
  validation, terminal-event validation, and shared conformance fixtures.
- EasyRemote owns Pipeline/EAL syntax and ergonomic `MissionRun.events()`.
- Live-tail Mission streaming remains incomplete until the daemon/ABI exposes a
  bounded stream contract. This slice covers page-based event observation only.

## Non-goals

- Do not change `docs/spec/daemon-sdk-requirements-v1.md`.
- Do not implement scheduler/retry policy or child Invocation semantics in
  EasyRemote.
- Do not fabricate event cursors or infer receipt refs from product fields.
