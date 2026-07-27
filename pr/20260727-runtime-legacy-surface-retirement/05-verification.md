# Verification

- `cargo fmt --check`
- `cargo build --bin easynet --bin easynet-daemon --bin easynet-keyring`
- `HOME=/tmp/easynet-clean-device-home.xO07f9 ./target/debug/easynet ability list --node easynet:///r/localhost/device/abae33c6-e4e3-40ac-8af0-e21b01e054b8 --format json`
  - verified 172 catalogue rows from the local runtime route.
  - verified rows retain canonical `descriptor_ref`.
- `cargo test --lib cli::daemon_client::remote_system_ability::tests::`
- `cargo test --lib ffi::invocation::tests::runtime_descriptor_resolver_prefers_local_catalog_for_runtime_owner`
- `git diff --check`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`

## Iteration 2

- `cargo fmt --check`
- `cargo test --lib support::platform::local_invoke::tests::runtime_device_read`
- `cargo test --lib cli::commands::groups::device::tests::`
- `cargo test --lib cli::commands::discover::tests::`
- Result: prototype rejected before commit because `tools/scripts/check-canonical-runtime-convergence-v2.sh` failed the runtime-state read subject boundary for `discover.rs`.
- Reverted production source changes from the prototype.

## Iteration 3

- `cargo build --bin easynet --bin easynet-daemon --bin easynet-keyring`
- `HOME=/tmp/easynet-clean-home.Zyfl0k ./target/debug/easynet start`
  - Result: clean HOME cannot start as a paired device without credentials; this confirmed the reported product path requires identity bootstrap and cannot be reproduced from empty state alone.
- `./target/debug/easynet runtime stop || true`
- `./target/debug/easynet leave --force --yes --purge-local-state || true`
  - Result: current user local EasyNet state root removed after explicit user authorization.
- `/Users/macbook.silan.tech/.local/bin/codegraph query "invocation.history.list meta.list_abilities descriptor_ref caller signer authority subject mismatch"`
- `npm test --prefix sdk/node`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo fmt --check`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`

## Iteration 4

- `/Users/macbook.silan.tech/.local/bin/codegraph query "URI terminology URA canonical runtime sdk receipt canonicalizer fail open governance subject"`
- `swift test` from `sdk/swift`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo fmt --check`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
