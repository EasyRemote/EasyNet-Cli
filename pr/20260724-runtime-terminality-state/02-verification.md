# Verification

## Commands

- `cargo test unknown_invocation_resource --features axon-pb`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `cargo fmt --check`
- `git diff --check`

## Result

All commands passed.

## Evidence

- Unknown stream cancel returns `ERR_INVALID_HANDLE`.
- Unknown stream close returns `ERR_INVALID_HANDLE`.
- Unknown bidi cancel returns `ERR_INVALID_HANDLE`.
- Unknown bidi close returns `ERR_INVALID_HANDLE`.
- Registered resource cancellation idempotency remains provider-owned.
- Registered bidi half-close idempotency remains `reserve_close_send_frame`-owned.
- SPEC v2 rejects legacy unknown-resource success paths.
