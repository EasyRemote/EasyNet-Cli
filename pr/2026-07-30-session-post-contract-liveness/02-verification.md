# Verification

- `cargo fmt --check`
- `cargo test established_session_silence_does_not_trigger_idle_timeout --lib`
- `cargo test session_contract_projection --lib`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- Full prior gate: `bash tools/scripts/check-sdk-cutover-readiness.sh`

