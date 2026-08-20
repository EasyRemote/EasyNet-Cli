# Architecture gate Axon layout closure

## Scope

Repair the architecture convergence gate so it runs against both the old
`core/runtime-rs/src/services/invocation` Axon layout used by fixtures and the
current checkout where runtime service code is no longer under that subdirectory.

## Invariants

- The gate must not fail before checking CLI source solely because an obsolete
  optional Axon service subdirectory is absent.
- The gate still requires the current Axon runtime root and Rust SDK root.
- Terminal receipt writer scanning should preserve old-layout coverage when the
  old invocation directory exists.
- No architecture rule is weakened for EasyNet-Cli production code.

## Verification

- `bash tests/scripts/test_check_architecture_convergence.sh` (full script is
  intentionally exhaustive; the run progressed past the canonical fixture into
  negative cases, then was interrupted after the parent bash stopped producing
  child-process progress)
- `/tmp/easynet-arch-canonical-fixture-check.sh` extracted from this test passed
  the canonical fixture check
- `bash tools/scripts/check-architecture-convergence.sh --root .`
- `git diff --check`
- `cargo fmt --check`
