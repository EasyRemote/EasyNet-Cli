# Intent

## Goal

Remove the daemon-local loopback compatibility path that derives an invocation
subject from the resolved callee when callers do not provide an explicit
subject.

## Non-goals

- Do not change the public `invoke_local_ability(ability, args)` API.
- Do not change product command behavior beyond earlier fail-closed readiness
  when the daemon has not published a control-plane identity.
- Do not add another route, subject, or receipt projection path.

## Acceptance criteria

- Local daemon loopback tuple construction has no `LocalDaemonSelf` subject
  policy.
- The generic local ability helper resolves the daemon identity before tuple
  construction and passes it as an explicit subject.
- Missing daemon identity fails before signed invocation construction.
- Convergence gates reject reintroduction of callee-as-subject loopback
  fallback.
