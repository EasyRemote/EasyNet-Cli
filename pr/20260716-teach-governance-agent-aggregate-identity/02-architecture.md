# Architecture

## Boundary

`src/daemon/persistence/agent_aggregate.rs` owns hosted Agent identity read projections. Governance teach code may translate projection errors into teach-domain messages, but it must not inspect `LocalAgentsFile` to resolve display names.

## Projection

`HostedAgentIdentityProjection` is a borrowed aggregate projection carrying:

- `profile`
- `name`
- `agent_ura`
- `signing_authority`

## Expected Effect

Teach/acquire/forget authority checks share the same hosted display-name semantics as Mission and local daemon delegation paths, while retaining the teach-domain admission and transaction logic for later state-machine convergence.
