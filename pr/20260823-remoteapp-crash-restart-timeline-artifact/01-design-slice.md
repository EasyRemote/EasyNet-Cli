# RemoteApp crash/restart timeline artifact gate

## Product seam

The crash/restart verifier required recovery event names and post-restart
fields, but event evidence was not causally ordered. A runner could report
`DAEMON_RESTARTED`, `SESSION_REHYDRATED`, `media_reattached`, and rendered
frames without proving that they happened in the same session timeline or in
the required order.

## Slice

- Require every crash/restart scenario to record `scenario_started_at_ms`.
- Require every lifecycle event to carry `at_ms`, selected Resource URA, and
  session id.
- Require lifecycle events to be strictly ordered by `at_ms`.
- Require daemon restart recovery to prove public `show_session`, watch-events
  reattachment, media reattachment, and first rendered frame occur after the
  preceding recovery phase.
- Require plugin worker restart to prove rendered frames occur after worker and
  target-monitor restart.
- Require terminal receipt replay and stale socket cleanup observations to be
  timestamped after the relevant crash/stale-socket events.

## Expected impact

This still does not prove live crash/restart product readiness without a real
runner artifact. It closes the evidence seam where unordered lifecycle facts
could be mistaken for deterministic session recovery.
