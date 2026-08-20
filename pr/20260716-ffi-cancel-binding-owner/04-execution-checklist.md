# Execution Checklist

- [x] Inspect current worktree and avoid unrelated dirty files.
- [x] Run CodeGraph-style search for `cancel_invocations_for_handle`.
- [x] Confirm production shutdown uses `cancel_invocations_for_binding`.
- [x] Remove the obsolete handle cleanup helper and no-feature stub.
- [x] Update the ownership test to call binding cleanup directly.
- [x] Run focused FFI cancellation tests.
- [x] Run compile warning check.
- [x] Run architecture convergence check.
- [x] Record verification results.
