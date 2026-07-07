# Boundary Proof

## SDK-Owned

- Mission request validation and JSON serialization.
- Typed projection DTOs for MissionStatus, MissionEvent, and MissionEventPage.
- Client lifecycle over an injected transport.
- TypeScript declarations for the same seam.

## Provider-Owned

- DescriptorRef selection for `mission.*` governed daemon abilities.
- EAL mission execution, status production, cancellation, and event log
  projection.
- Receipt production and child Invocation fact anchoring.

## Product-Owned

- EasyRemote Pipeline DSL and decorators.
- Scheduling, retries, mission UX, and operator-facing status presentation.
- Filesystem admission policy for run-file inputs.

## Rejected Designs

- Node reading run-file contents: rejected because run-file source loading and
  filesystem policy belong to daemon/provider implementations.
- SDK-side MissionPlan child Invocation validation: rejected for this Node seam
  because the shared case is not required for Node and would duplicate the P0
  Mission plan semantics.
- Path-like mission IDs: rejected because mission IDs are opaque run refs, not
  filesystem selectors.
