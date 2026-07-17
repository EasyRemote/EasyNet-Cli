# Verification

## Focused

- `cargo fmt --all -- --check`
- `cargo test --lib daemon::axon_bridge::dispatch_shim::tests -- --nocapture`

## Architecture

- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tests/scripts/test_check_architecture_convergence.sh`
- `PYTHON_BIN=/opt/anaconda3/bin/python bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `PYTHON_BIN=/opt/anaconda3/bin/python bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`

## Required Negative Evidence

- A trusted-local wire envelope without caller is rejected.
- A trusted-local wire envelope without subject is rejected.
- A trusted-local wire envelope without a valid 16-byte nonce is rejected.
- An unsealed local-system dispatch is rejected.
- A trusted-local classification carrying a non-system caller is rejected.
- The architecture gate rejects local-system fallback logic in
  `wire_descriptor.rs`.
- The architecture gate rejects a direct local-system factory ingress outside
  `SystemInvocationIssuer`.
- The architecture gate rejects a public or unsealed local-system wire
  constructor.

## Results

- `cargo fmt --all -- --check`: passed.
- `cargo test --lib daemon::axon_bridge::wire_descriptor::tests -- --nocapture`:
  4 passed.
- `cargo test --lib daemon::axon_bridge::dispatch_shim::tests -- --nocapture`:
  12 passed.
- `cargo test --lib daemon::boot::kernel::tests -- --nocapture`: 9 passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: all failure
  fixtures passed.
- `PYTHON_BIN=/opt/anaconda3/bin/python bash tools/scripts/check-canonical-runtime-convergence-v2.sh`:
  passed.
- `PYTHON_BIN=/opt/anaconda3/bin/python bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`:
  passed.
