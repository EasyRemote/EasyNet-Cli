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
- `EASYNET_RELEASE_PROFILE=debug bash packaging/release/build-release-tarball.sh`
  - passed; produced
    `target/release-tarball/easynet-aarch64-apple-darwin.tar.gz` without
    shipping `axon-runtime`.
- `EASYNET_TEST_BACKEND_NO_BUILD=1 EASYNET_TEST_BACKEND_PORT=18080 EASYNET_TEST_HUB_TLS_PORT=15443 bash packaging/release/e2e-release-flow.sh`
  - passed against the Docker EasyNet backend/Hub using the release-shape
    sandbox install.
  - The harness used `device join --boot no` to keep the measured transition on
    `runtime start`. This is a test-isolation choice; the product default still
    remains join-and-start.
  - Backend SSE emitted device-online invalidation in 1227 ms; the backend
    `/api/v1/devices` read model returned `ONLINE` in 52 ms after that
    invalidation.
  - Graceful `runtime stop` emitted backend SSE `removed` in 235 ms; the
    backend read model reported `UNKNOWN` for the device afterward.
  - Restart restored backend product presence, local ability listing passed,
    hosted-agent URAs were minted for `dev.consent-default`, `dev.codex`, and
    `dev.claude`, and the structured advertise prelude reported all five
    hosted agents.
  - Abrupt `SIGKILL` of the sandbox `easynet-daemon` emitted backend SSE
    `removed` in 143 ms; the backend read model reported `UNKNOWN` afterward.
  - The flow cleaned up the Docker backend stack and left no matching host
    `easynet-daemon` process.

External product gates closed:

- Backend SSE/read-model product presence propagation is covered by the
  release-flow gate above.
- Full process-level graceful stop and abrupt-kill propagation budgets are
  covered against a running backend/Hub by the same release-flow gate.
