# Verification

Passed:

```text
cargo test -p easynet same_generation_can_register_same_hosted_agent_on_distinct_devices
cargo test -p easynet advertise_abilities_payload_carries_generation
cargo test -p easynet remote_caller_defaults_to_discovered_hub_daemon_identity_without_credentials
bash tools/scripts/check-architecture-convergence.sh
bash tests/scripts/test_check_architecture_convergence.sh
git diff --check -- src/daemon/federation/advertise.rs src/daemon/persistence/federation_revoke.rs src/daemon/invocation/bidi/session_initiator/prelude.rs src/daemon/invocation/routing/remote_invoke.rs pr/20260716-hosted-agent-revoke-fence
```
