# Runtime Lifecycle Verification

Planned checks:

- `bash tests/scripts/test_check_project_structure_v1.sh` - passed.
- `cargo test --lib --features axon-pb daemon::boot::lifecycle` - passed
  (18 tests).
- `cargo test --lib --features axon-pb daemon::boot::lifecycle` - passed
  after service collaborator refactor (19 tests).
- `cargo test --lib --features axon-pb supervisor_reconnects_when_hub_starts_after_cli_daemon`
  - passed.
- `cargo test --lib --features axon-pb invoke_stream_subscribe_directory_v2_emits_heartbeat_when_idle`
  - passed.
- `cargo check --lib --features axon-pb` - passed with no warnings.
- `bash tests/scripts/test_check_release_package_contract.sh` - passed.
- `cargo test --test script_checks release_package_contract_script_holds` -
  passed.
- Hermetic hub-mode lifecycle soak - passed 50 cycles:
  `runtime start --as-hub` + `runtime status --json` running assertions +
  `runtime stop` + `runtime status --json` stopped assertions, using debug
  `easynet`/`easynet-daemon`, self-signed TLS, `listen_tcp = 127.0.0.1:0`,
  fixed sandbox Pages port, and a temporary `HOME`.
- `cargo test --lib --features axon-pb handle_heartbeat_renews_owner_projection_lease`
  - passed.
- `cargo test --lib --features axon-pb heartbeat_includes_owner_projection_refresh_batch`
  - passed.

External/manual gates still required for full product release confidence:

- Backend SSE/read-model product presence propagation test.
- Graceful stop and abrupt kill propagation budgets against a running Hub.
