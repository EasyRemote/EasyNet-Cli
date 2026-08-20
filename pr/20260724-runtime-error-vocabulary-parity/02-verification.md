# Verification

## Commands

- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `cargo test native_runtime_signer_error_records_caller_signer_projection --features axon-pb`
- `cargo fmt --check`
- `git diff --check`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`

## Result

- SPEC v2 self-test passed.
- Focused FFI/native runtime signer projection regression passed.
- Rust formatting passed.
- Whitespace diff check passed.
- SPEC v2 main gate passed.

## Architectural delta

Native RuntimeHandle signer-custody failures now project through the same
canonical caller-signer vocabulary used by remote invocation:

- ABI return code: `ERR_PERMISSION_DENIED`
- typed runtime error code: `CALLER_SIGNER_UNAVAILABLE`
- stage: `caller_identity`
- retry: `never`

Raw KeyService/keyring implementation detail is not part of the
product-visible error surface.
