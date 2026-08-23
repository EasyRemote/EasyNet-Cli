# RemoteApp input observer-independence artifact gate

## Product seam

The input injection verifier already requires applied pointer/keyboard events
and an `os_effect` object observed after `host_applied_at_ms`. That still leaves
a proof gap: the same injection path could set `observed=true` without a
separate platform observer proving the OS actually changed state for the
specific input event and focused target epoch.

## Slice

- Require every applied input frame to expose a stable `input_event_id`.
- Require each OS effect to bind that `input_event_id`.
- Require every OS effect to declare `observer_independent_from_injector=true`.
- Require a platform `target_focus_epoch` and bind each OS effect to that
  epoch.
- Keep existing latency, geometry, pointer tolerance, keyboard focus, stale
  sequence, consent, and terminal receipt checks unchanged.

## Expected impact

This does not implement live OS input injection. It closes the evidence seam
where product readiness could be inferred from injection-path telemetry rather
than independent platform observation of the actual pointer/key effect.
