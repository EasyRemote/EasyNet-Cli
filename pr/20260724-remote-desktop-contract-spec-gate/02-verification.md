# Verification

Executed checks:

- `bash tools/scripts/check-remote-desktop-contract-boundary.sh`
- `bash tests/scripts/test_check_remote_desktop_contract_boundary.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `cargo fmt --check`
- `cargo test canonical_runtime_convergence_v2_script_contract_holds --test script_checks`
- `cargo test remote_desktop_contract_boundary_script_holds --test script_checks`

Outcome: all passed. The SPEC v2 self-test now carries a temporary
`alias = "web_rtc"` fixture and must reject it before reporting success.
