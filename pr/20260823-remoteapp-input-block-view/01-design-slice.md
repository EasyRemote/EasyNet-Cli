# RemoteApp input runtime block session-view slice

## Product requirement

When a RemoteApp session loses host input execution permission after media is
already active, every public session projection must continue to show the same
input-only blocker. The UI must not depend only on the transient watch event
that observed the failure.

## Boundary

- The RemoteApp plugin owns the session-local input execution state.
- Target tracking remains responsible only for target availability/focus/scope.
- The frontend consumes `input_readiness`; it must not reconstruct daemon
  runtime state from event history.

## Implemented slice

- Store the last runtime input permission block reason on the session aggregate.
- Project that reason through `input_readiness.blocked_reason` and
  `input_plane.readiness`.
- Clear the session-local runtime block only after input is reactivated for the
  current transport epoch.
- Add view/session regressions plus lifecycle/product gates.

## Non-claims

- This does not prove live macOS Accessibility revoke E2E.
- This does not implement OS input injection.
- This does not make target-scoped window/application input product-ready.
