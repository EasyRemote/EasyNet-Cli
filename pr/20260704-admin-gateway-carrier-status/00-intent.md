# Intent

## Goal

Implement the next Daemon SDK v1 Admin + Gateway profile slice without changing
`docs/spec/daemon-sdk-requirements-v1.md`.

This slice adds Admin + Gateway carrier/projection support:

- Build complete Invocation JSON carriers for daemon-owned `agent.list`,
  `agent.start`, `agent.stop`, `agent.refresh`, and `session.list`.
- Project daemon lifecycle/status JSON into a typed `GatewayStatus` DTO that
  distinguishes control liveness, Invocation readiness, directory/runtime
  readiness, public listener hints, and trust readiness.
- Project daemon `agent.list` rows and lifecycle mutation results into typed
  SDK admin DTOs.
- Align the Go C ABI Admin facade with the Python C ABI facade for
  `GatewayStatus`: attach to the daemon lifecycle handle, read
  `easynet_daemon_status`, and delegate readiness classification to the shared
  Rust Admin + Gateway projection.

## Non-Goals

- Do not implement backend pairing-token HTTP calls.
- Do not implement ACME, certificate provisioning, or onboarding UX.
- Do not implement complete device-session CRUD beyond `session.list` carrier.
- Do not add one-method-per-ability transport outside Runtime Core.
- Do not change the requirements SPEC.
- Do not synthesize Hub join/leave, pairing, credential, revoke, or
  session-create/delete mutation results from read-model status facts.

## Acceptance Criteria

- C ABI exposes Admin + Gateway helper functions with the same handle, pointer,
  UTF-8, JSON, and caller-owned string rules as existing profile projections.
- Shared Rust contract owns gateway status and agent record DTO semantics.
- Agent lifecycle carriers preserve the complete Invocation tuple and lower to
  daemon-owned system abilities.
- Gateway status projection preserves degraded states instead of collapsing
  control-only or invocation-down daemons into `ready=false` without detail.
- SDK schemas, fixtures, conformance case, ABI header/spec, and feature
  discovery are updated in the same semantic slice.
- Focused unit tests cover carrier construction, invalid target validation,
  gateway status projection, agent rows, lifecycle results, and invalid handles.
- Go C ABI Admin `GatewayStatus` no longer reports NotImplemented when daemon
  lifecycle status is available through `easynet_daemon_status`.
