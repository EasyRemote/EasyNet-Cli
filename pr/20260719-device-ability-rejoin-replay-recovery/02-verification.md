# Verification

- Unit tests for stale foreign-authority cleanup in `DeviceAbilityStore`.
- Unit tests for boot replay diagnostic detail in `easynet-daemon`.
- Targeted cargo test for device ability registrar/store replay behavior.

## Results

- `cargo test daemon::ability::builtins::device_control::ability_management::store::tests::quarantine_unhosted_device_authority_hides_old_device_rows_from_replay --features axon-pb`
- `cargo test daemon::ability::builtins::device_control::ability_management::registrar::tests::boot_replay_quarantines_previous_device_authority_rows --features axon-pb`
- `cargo test --bin easynet-daemon device_replay_boot_policy --features axon-pb`
- `cargo test daemon::ability::builtins::device_control::ability_management --features axon-pb`
- `cargo build --bin easynet --bin easynet-daemon --features axon-pb`
- `target/debug/easynet start`
- `cargo fmt --check`
- `cargo check --bin easynet --bin easynet-daemon`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `target/debug/easynet invocation list --limit 3 --format json`

The local daemon was restarted from `target/debug/easynet` on 2026-07-19 after
the current build. The runtime remained `running` while product presence was
`session_admitted=false` and `directory_status=suspect`; `easynet invocation
list --limit 3 --format json` still returned completed local invocations with
verified receipt chains. This proves local invocation ledger access does not
depend on stale remote directory presence for the current device authority.

Browser-tool verification was attempted against `http://127.0.0.1:8080/`,
`http://localhost:8080/`, and `http://test.dev.pages.localhost:8787/` using the
Codex in-app browser. The browser runtime rejected those localhost navigations
with `net::ERR_BLOCKED_BY_CLIENT`, so browser evidence is intentionally not
used as acceptance evidence for the local daemon path.

The real local store had 17 installed rows owned by previous device
`easynet:///r/localhost/device/555e535e-6cee-4e13-bd78-4475701aa08f`.
Boot quarantined those rows and reached `ready`.
