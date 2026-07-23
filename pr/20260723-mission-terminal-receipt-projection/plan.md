# Mission terminal receipt projection convergence

## Goal

Remove the Mission/EAL receipt graph compatibility wrapper
`"receipt": {"anchor": ...}` and project the child Invocation terminal receipt
as an explicit `terminal_receipt` field.

## Root abstraction problem

Mission child invocation records already carry one canonical terminal receipt
and a list of dependency receipt anchors. Wrapping the terminal receipt under a
generic `receipt.anchor` field creates a product-specific receipt shape that is
inconsistent with the rest of the canonical runtime vocabulary and easy to
confuse with the retired InvocationResult `receipt` alias.

## Invariants

1. Mission invocation records expose the child terminal receipt as
   `terminal_receipt`.
2. Dependency receipt anchors remain under `dependency_receipts`.
3. No `MissionInvocationRecord` projection emits top-level `receipt`.
4. EAL receipt graph substitution still carries completed child invocation
   records.
5. This slice does not change Axon receipt finalization or Mission child
   invocation execution semantics.

## Boundary proof

- The daemon Mission gateway owns the product-level trace projection.
- Axon/LocalRuntime owns canonical receipt creation and finalization.
- The projection should name the already-finalized child receipt explicitly
  instead of preserving a product wrapper that looks like a legacy receipt
  alias.

## Verification plan

- Mission invocation gateway focused tests.
- EAL interpreter focused receipt graph tests.
- Canonical runtime convergence v2 gate.
- Architecture convergence gate.
- Repository formatting.
- Codegraph sync/status.

## Decisions

- Rename the Mission trace projection field to `terminal_receipt` instead of
  preserving the generic `receipt.anchor` wrapper.
- Keep `dependency_receipts` unchanged because it already names the receipt
  graph edge set explicitly.
- Limit this slice to Mission/EAL trace projection. Runtime receipt
  finalization, LocalRuntime admission, and child invocation execution remain
  unchanged.

## Delta

- `MissionInvocationRecord.projection()` now emits `terminal_receipt`.
- Gateway regression coverage now asserts the retired top-level `receipt`
  wrapper is absent.
- SPEC v2 now rejects reintroducing the Mission `receipt.anchor` wrapper.

## Results

- `cargo fmt --check` passed.
- `cargo test child_is_receipt_anchored_and_inherits_subject_trace_and_parent_deadline`
  passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` passed.
- `cargo test loop_steps_retain_dependency_receipts_and_trace_id` passed.
- `tools/scripts/check-architecture-convergence.sh` passed.
- `tools/scripts/check-sdk-canonical-public-api.sh` passed.
- `codegraph sync . && codegraph status .` completed with an up-to-date index.
