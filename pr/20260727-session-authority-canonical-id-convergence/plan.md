# Session Authority Canonical ID Convergence

## Goal

Remove the remaining retired invocation-history subject compatibility predicates from Rust, Java, and Swift runtime authority paths. Session authority admission must reject obsolete history carriers because their session id is non-canonical, not because product code preserves a retired path-specific helper.

## Boundary Invariants

1. Session authority is a canonical runtime authority concept, not an EasyNet or EasyRemote receipt compatibility concept.
2. A session authority subject may be either a canonical User URA or a user-owned `resource/user.<id>/session/<session_id>` URA.
3. `session_id` must be non-empty and contain only `[A-Za-z0-9.-]`; underscore and colon carriers are rejected structurally.
4. Runtime-state reads use `runtime-state/read`; invocation-history-specific retired paths are not modeled as a live compatibility state.
5. Public tuple validation must not preserve a retired invocation-history predicate as an admissible helper.

## Refactoring Plan

1. Move Rust rejection from `is_retired_invocation_history_subject*` into canonical session-id parsing.
2. Delete Rust retired subject constant, predicate, and predicate-specific test.
3. Add Java/Swift canonical session-id validation matching Go/Python/Node.
4. Delete Java/Swift retired subject constants and predicates.
5. Update tests to assert canonical session-id rejection rather than path compatibility rejection.
6. Extend SPEC v2 gate so Rust/Java/Swift cannot reintroduce retired invocation-history subject helpers.

## Verification

- Targeted Rust authority/FFI/core tests.
- Java runtime seam tests if local toolchain is available.
- Swift runtime seam tests if local toolchain is available.
- `cargo fmt --check`.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`.
