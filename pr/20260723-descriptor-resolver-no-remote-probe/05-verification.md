# Verification

Executed checks:

- `cargo test runtime_descriptor_resolver --features axon-pb`
  - Result: pass, 6 resolver tests.
- `cargo test descriptor_resolution_errors_project_canonical_runtime_codes --features axon-pb`
  - Result: pass, 1 error-projection test.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - Result: pass, `canonical-runtime-convergence-v2: OK`.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - Result: pass, `canonical-runtime-convergence-v2 self-test ok`.
- `tools/scripts/check-architecture-convergence.sh`
  - Result: pass, `architecture-convergence: OK`.
- `cargo fmt --check`
  - Result: pass.

Regression evidence:

- Descriptor misses for remote owners now fail as bounded
  `DESCRIPTOR_NOT_FOUND` realm catalog misses.
- The resolver no longer exposes caller-signer, owner-offline, or remote
  transport failure states.
- The obsolete typed remote submit wrapper used by the removed probe path is
  gone.
