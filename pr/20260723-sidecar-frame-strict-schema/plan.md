# Sidecar frame strict schema convergence

## Goal

Remove the sidecar IPC compatibility surface where request/response frames and
the daemon invocation envelope silently ignored unknown JSON fields.

## Root abstraction problem

The sidecar frame model is a process-boundary protocol, but its serde model was
permissive. A plugin process could emit a response with extra fields and believe
the host accepted those facts, while the daemon silently discarded them before
receipt/finalization. That creates a hidden protocol fork and makes plugin
ecosystem behavior depend on undocumented fields.

## Invariants

1. Sidecar request frames reject unknown top-level and variant fields.
2. Sidecar response frames reject unknown top-level and variant fields.
3. Sidecar invocation envelopes reject unknown identity/context fields.
4. Ability payloads remain open only inside `args`/`frame`/`value`, where the
   selected ability schema owns interpretation.
5. Decode failures stay typed as `SidecarFrameDecodeFailed` before runtime
   mutation or stream emission.

## Verification plan

- Focused sidecar tests for request, response, and nested invocation unknown
  field rejection.
- Sidecar host tests to ensure strict response decode still reports typed errors.
- `cargo fmt --check`.
- SPEC v2 convergence gate.
- Architecture convergence gate.
- codegraph sync/status.

