# RemoteApp target geometry event batch plan

## Intent

Preserve both target move and target resize facts when one host target
observation changes both origin and dimensions.

## Boundary

- Do not make platform observers emit duplicate observations.
- Keep `RemoteAppTargetBindingStateMachine` as the single committed target
  lifecycle writer.
- Keep one target snapshot mutation per host observation.
- Preserve existing public event JSON shape; only add additional ordered events
  when the same committed geometry update proves more than one lifecycle fact.

## Invariants

- One geometry observation updates `target_geometry_revision` once.
- If only origin changes, emit `TARGET_MOVED`.
- If only size changes, emit `TARGET_RESIZED`.
- If origin and size both change, emit ordered `TARGET_MOVED` then
  `TARGET_RESIZED` with the same committed revision.
- Session event log sequence numbers remain monotonic and are assigned only by
  `RemoteDesktopEventLog`.

## Verification plan

- Add target state-machine regression for combined move+resize.
- Add session aggregate regression proving watch-events/event-log sees both
  ordered events with monotonic sequences.
- Run targeted Rust tests.
- Run RemoteApp lifecycle/input boundary scripts.
- Run EasyNet Frontend targeted remote desktop tests.
- Run `git diff --check`, CodeGraph, and URA-only scan.
