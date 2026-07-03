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
- `cargo test --test cross_device_invoke_remote_e2e session_graceful_close_emits_stream_closed_offline`
  - passed.
- `cargo test --test cross_device_invoke_remote_e2e subscribe_directory_stream_tracks_real_session_online_and_offline`
  - passed.
- `cargo test --lib --features axon-pb invoke_stream_dispatches_subscribe_directory_v2_emits_directory_events`
  - passed.
- `cargo test --test cross_realm_directory_streaming_e2e streaming_chain_propagates_presence_remove`
  - passed.

External/manual gates still required for full product release confidence:

- Backend SSE/read-model product presence propagation test.
- Full process-level graceful stop and abrupt-kill propagation budgets against
  a running backend/Hub. Daemon-local session close and directory-stream
  propagation are covered above; the remaining gate needs the external product
  subscriber/read-model path.
