# Implementation Plan

1. Refactor daemon boot identity loader to call `load_runtime_caller_signer`.
2. Refactor federation trust auto-wire to call `load_runtime_caller_signer`.
3. Add/adjust tests that assert upper-layer paths no longer import or call the concrete runtime-owner loader.
4. Run focused tests and convergence gates.

