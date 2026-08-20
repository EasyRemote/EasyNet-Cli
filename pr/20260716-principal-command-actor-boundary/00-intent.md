# Principal Command Actor Boundary

## Goal

Converge PrincipalLifecycle CLI command construction on an explicit actor-source
boundary. The CLI may keep source-compatible self-authorization behavior for
bootstrap, enrollment, and first-key flows, but the command serializer must not
silently fall back from a missing `actor_ura` to `principal_ura`.

## Non-goals

- No PrincipalLifecycle daemon state-machine redesign.
- No public CLI flag removal.
- No SDK Go/Python PrincipalLifecycle API change.
- No change to daemon admission semantics or receipt output.

## Acceptance Criteria

- `principal_command` receives an explicit actor object, not an optional string.
- Subject-self authorization is represented as a named state.
- Existing CLI JSON payloads remain compatible.
- The architecture convergence gate rejects reintroducing the hidden fallback.
- Targeted PrincipalLifecycle CLI tests pass.
