# Host Binding Lifecycle Provider Plan

## Goal

Move Host Binding from codec/hash-only seam toward the SPEC-owned runtime model:
the SDK owns host-stream binding DTOs, envelope/frame/hash codecs, readiness and
cleanup contracts, and the lifecycle state machine. Product repositories only
provide endpoint readiness probes and cleanup behavior through generic provider
interfaces.

## Boundary Proof

- SDK-owned:
  - Host-stream binding request/projection DTOs.
  - Request envelope decoding.
  - Item/error/terminal frame encoding.
  - Output hash folding and cursor validation.
  - Readiness/cleanup DTO projection.
  - Lifecycle provider interface and deterministic state transitions.
- Product-owned:
  - User function execution.
  - Decorator semantics.
  - Product host process startup and thread model.
  - Product-specific cleanup policy selection.

## Implementation Steps

1. Add Python Host Binding lifecycle provider, controller, and state enum.
2. Add Go Host Binding lifecycle provider, controller, and state enum.
3. Keep existing codec/hash public behavior intact.
4. Add provider-backed readiness/cleanup tests in both languages.
5. Update SDK parity documents only after both language implementations and tests
   prove the same state model.

## Verification

- Python Host Binding tests.
- Python SDK tests.
- Go Host Binding tests.
- Go SDK tests.
- SDK parity matrix checks.
- Formatting and diff hygiene.
