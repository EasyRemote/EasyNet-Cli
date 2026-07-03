# Runtime Lifecycle Verification

Planned checks:

- `bash tests/scripts/test_check_project_structure_v1.sh` - passed.
- `cargo test --lib --features axon-pb daemon::boot::lifecycle` - passed
  (18 tests).
- `cargo test --lib --features axon-pb supervisor_reconnects_when_hub_starts_after_cli_daemon`
  - passed.
- `cargo test --lib --features axon-pb invoke_stream_subscribe_directory_v2_emits_heartbeat_when_idle`
  - passed.
- `cargo check --lib --features axon-pb` - passed with no warnings.
- `bash tests/scripts/test_check_release_package_contract.sh` - passed.
- `cargo test --test script_checks release_package_contract_script_holds` -
  passed.

External/manual gates still required for full product release confidence:

- 50-cycle sandbox `easynet runtime start` / `runtime stop` process test.
- Backend SSE/read-model product presence propagation test.
- Graceful stop and abrupt kill propagation budgets against a running Hub.
- Heartbeat lease renewal test after owner-projection TTL.
