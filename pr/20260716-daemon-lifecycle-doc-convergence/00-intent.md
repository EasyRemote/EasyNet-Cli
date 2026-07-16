# Intent

## Goal

Converge daemon lifecycle and invocation-boundary documentation with the current
source architecture: product calls enter `daemon.sock`, admitted local calls run
inside the daemon's embedded Axon `LocalRuntime`, and retired callback/lifecycle
surfaces are not described as active architecture.

## Non-goals

- Do not modify runtime behavior in this slice.
- Do not stage SDK conformance rewrites, mission ownership notes, URI/URA
  cleanup, RFC backup deletion, or Rust formatting churn.
- Do not revive any `runtime_dispatch` compatibility path.

## Acceptance Criteria

- Active architecture docs no longer describe `runtime-dispatch` or `GatewayApi`
  as current product lifecycle or execution boundaries.
- Runtime lifecycle docs reject retired bridge state instead of preserving a
  legacy lifecycle class.
- Wrapper/runtime refactor spec names `JoinConnectionSnapshot` as the daemon
  status source rather than a separate federation-init probe.
- Verification proves the retired source files are absent and the current gates
  still pass.
