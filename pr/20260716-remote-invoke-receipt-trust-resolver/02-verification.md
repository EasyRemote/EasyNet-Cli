# Verification

Passed:

```text
cargo test -p easynet remote_receipt_key_resolver_rejects_malformed_trust_anchor
cargo test -p easynet remote_caller_defaults_to_discovered_hub_daemon_identity_without_credentials
bash tools/scripts/check-architecture-convergence.sh
bash tests/scripts/test_check_architecture_convergence.sh
git diff --check -- src/daemon/invocation/routing/remote_invoke.rs pr/20260716-remote-invoke-receipt-trust-resolver
```
