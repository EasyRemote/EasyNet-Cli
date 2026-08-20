# Verification

## 2026-07-07

- `cargo test invocation_sign_prepared_local --lib`: passed.
- `cargo test invocation_prepare_and_sign_prepared_allocate_state_handles --lib`: passed.
- `cargo test invocation --lib`: passed.
- `go test .` in `sdk/go`: passed.
- `go test -tags 'easynet_cabi' .` in `sdk/go`: passed.
- `PYTHONPATH=.:tests python -m unittest tests.test_cabi` in `sdk/python`: passed.
- `bash tools/scripts/check-sdk-parity-matrix.sh`: passed.
- `bash tools/scripts/check-sdk-cutover-readiness.sh`: passed until the known
  backend SDK-only boundary gate.

Covered facts:

1. `SignedInvocation` preserves signer policy proof.
2. C ABI exposes `easynet_invocation_sign_prepared_local`.
3. Local signing uses the daemon default keyring and preserves signer policy
   proof in signed JSON.
4. Local signing failure preserves the prepared handle.
5. Go and Python C ABI transports select local daemon signing from
   `SignedInvocation.policy.mode` instead of smuggling it through
   `signature_json`.
