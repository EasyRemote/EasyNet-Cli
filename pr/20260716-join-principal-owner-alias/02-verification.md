# Verification

Passed:

```text
cargo test -p easynet federation_join_with_principal_proof_binds_device_owner_in_runtime_trust
bash tools/scripts/check-architecture-convergence.sh
bash tests/scripts/test_check_architecture_convergence.sh
git diff --check -- src/daemon/invocation/dispatch/unary_dispatcher.rs src/daemon/invocation/dispatch/daemon_invocation_service_tests/unary.rs pr/20260716-join-principal-owner-alias
```
