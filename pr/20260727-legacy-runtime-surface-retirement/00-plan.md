# Legacy runtime surface retirement

## Intent

Continue canonical runtime convergence by removing legacy or compatibility
surfaces that preserve a second authority path, a second DTO shape, or a hidden
Invocation tuple default after the SPEC v2 gate is already green.

## Boundary invariants

- Public product ingress must preserve the seven Invocation tuple fields instead
  of silently constructing an underspecified call.
- Compatibility fields are only allowed when the SPEC explicitly requires them.
- Runtime and SDK boundaries must use generic runtime concepts, not EasyNet or
  EasyRemote product vocabulary.
- Receipt and descriptor validation must reject retired shapes instead of
  accepting and normalizing them.
- Existing docs/spec edits outside this plan are not part of this iteration.

## Current evidence to gather

- Use codegraph and source search to locate remaining `legacy`, `compat`,
  `fallback`, `default subject`, and tuple-defaulting surfaces.
- Prefer producers and public ingress adapters over tests/docs-only findings.
- Confirm any chosen deletion with a regression gate so the retired path cannot
  return unnoticed.

## Execution plan

1. Build a source inventory of remaining legacy/compat runtime surfaces.
2. Select the smallest high-value root that removes a real alternative authority
   or tuple-defaulting path.
3. Refactor the owner module rather than adding caller-side patches.
4. Remove obsolete code and migrate callers.
5. Verify with targeted tests and SPEC v2 gate.

## Verification log

- `cargo fmt --check`
- `cargo test -q traditional_call_requires_explicit_device_target --features axon-pb`
- `cargo test -q traditional_call_unchanged --features axon-pb`
- `cargo test -q deps_inferred --features axon-pb`
- `tools/scripts/check-eal-interpreter-flat-call-boundary.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-architecture-convergence.sh`
