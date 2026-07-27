# Pending Dispatch Offline Cutover Plan

Date: 2026-07-27

## Goal

Remove the remaining no-target pending-dispatch registration compatibility seam so every production remote unary/stream/bidi waiter is bound to an explicit target URA and can be terminated by presence/offline events.

## Root Abstraction Problem

`PendingDispatchMap` and `PendingStreamDispatchMap` still exposed `register_pending()` as a no-target registration path. The method delegated to `register_pending_for("")` and documented the old behavior: calls that were not wired to cancel-on-offline would wait until the oneshot dropped or the request timeout fired.

That is a lifecycle fork. Remote invocation/session lifecycle has an explicit state machine with terminal offline cancellation; a no-target pending entry cannot participate in that state machine.

## Boundary Invariants

1. Every production pending unary dispatch must carry the target execution URA.
2. Every production pending stream/bidi dispatch must carry the target execution URA.
3. Offline presence events must release matching outstanding waiters immediately.
4. Tests must exercise the explicit-target API directly; they must not keep a test-only compatibility surface alive.
5. No caller may rely on an empty target URA to mean "not wired yet".

## Codegraph Evidence

- Production remote unary path uses `pending.register_pending_for(&selected_route.execution_host_ura)`.
- Production remote stream path uses `pending.register_pending_for(&selected_route.execution_host_ura)`.
- Existing bare `register_pending()` call sites are inside `pending_dispatch.rs` unit tests.

## Implementation Direction

1. Delete `PendingDispatchMap::register_pending()`.
2. Delete `PendingStreamDispatchMap::register_pending()`.
3. Update unit tests to register with explicit target URAs.
4. Keep `register_pending_for` and `register_lossless_pending_for` as the only lifecycle entrypoints.
5. Reject blank target URAs at the registration boundary instead of encoding a compatibility sentinel.
6. Add a dedicated boundary gate so the no-target registration surface cannot return.

## Acceptance Checks

- `rg 'register_pending\\(' src tests sdk` has no live call sites.
- Pending-dispatch unit tests pass.
- `bash tools/scripts/check-pending-dispatch-target-boundary.sh` passes.
- `bash tests/scripts/test_check_pending_dispatch_target_boundary.sh` passes and rejects restored compatibility methods, missing target guards, and production callers that do not bind the selected execution-host URA.
- `cargo test -q --features axon-pb pending_dispatch_target_boundary_script_contract_holds` passes.
- Script/SPEC gates continue to pass.

## Implemented Delta

- Removed the unary and stream no-target pending registration entrypoints.
- Added `require_pending_target_ura` as the single registration guard for unary and stream pending dispatch tables.
- Migrated pending-dispatch tests to explicit target URAs and added blank-target panic tests.
- Added `tools/scripts/check-pending-dispatch-target-boundary.sh`.
- Added `tests/scripts/test_check_pending_dispatch_target_boundary.sh`.
- Wired the new boundary gate into `tests/script_checks.rs`.
- Wired the new boundary gate into `tools/scripts/check-canonical-runtime-convergence-v2.sh` through `check_daemon_tuple_route_contract`.

## Verification Log

- `bash tools/scripts/check-pending-dispatch-target-boundary.sh` — pass.
- `bash tests/scripts/test_check_pending_dispatch_target_boundary.sh` — pass.
- `cargo test -q --features axon-pb pending_dispatch_target_boundary_script_contract_holds` — pass.
