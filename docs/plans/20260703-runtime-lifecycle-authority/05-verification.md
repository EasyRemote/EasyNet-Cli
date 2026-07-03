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
