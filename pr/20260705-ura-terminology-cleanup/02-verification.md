# Verification

Planned checks:

- `cargo test runtime_dispatch --lib`
- `cargo test send_task --lib`
- `cargo test federation_probe --lib`
- `cargo test federation::resolver --lib`
- `go test ./...`
- `cargo fmt --check`
- `git diff --check`
- Targeted scan for retired implementation/test address wording outside
  historical docs, generated protobuf files, and HTTP request-target usage.

Completed checks:

- `cargo fmt`
- `go test ./...`
- `cargo test runtime_dispatch --lib`
- `cargo test send_task --lib`
- `cargo test federation_probe --lib`
- `cargo test federation::resolver --lib`
- `cargo test keyring --lib`
- `cargo test backend_identity_reader --lib`
- `cargo test agent_id --lib`
- `cargo test invoke_remote_initiator --lib`
- `cargo fmt --check`
- `git diff --check`
- Targeted scan for retired implementation/test identity spellings returned no
  matches outside excluded docs/generated files.
