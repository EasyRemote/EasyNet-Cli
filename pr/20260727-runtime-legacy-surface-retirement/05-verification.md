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
