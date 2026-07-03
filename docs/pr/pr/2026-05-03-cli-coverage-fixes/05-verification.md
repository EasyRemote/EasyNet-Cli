# Verification

## Source-level verification

- `cargo test --features axon-pb -q --lib --no-run`
  - PASS
- `cargo test --features axon-pb -q --lib ability_item_fallback_maps_federation_descriptor_shape -- --nocapture`
  - PASS
- `cargo test --features axon-pb -q --lib local_exec_target_accepts_literal_local -- --nocapture`
  - PASS
- `cargo test --features axon-pb -q --lib decode_exec_stream_falls_back_to_plain_text -- --nocapture`
  - PASS
- `cargo test --features axon-pb -q --lib resolved_device_record_keeps_cross_tenant_realm_and_abilities -- --nocapture`
  - PASS

## Fixture-level verification

- `bash /tmp/cli-coverage-final.sh`
  - Still reports the old 44/47 table.
  - Root cause: the script executes `easynet` inside the existing docker
    fixture containers, and those binaries still print the old
    `fleet.exec_remote` path / old `auth abilities` behavior.
  - Evidence:
    - `ability exec same-hub` failure still says `invoke fleet.exec_remote`
    - `auth abilities X-HUB` failure still surfaces raw backend 404
  - Conclusion: fixture binary not rebuilt/redeployed from the patched source;
    this run does not invalidate the source fix.

## Known unrelated repo state

- `cargo test --features axon-pb -- --list` hits pre-existing integration-test
  failures outside this patch:
  - `tests/federation_real_invoke.rs`: stale `Credentials { realm: ... }`
  - `tests/cross_device_invoke_remote_e2e.rs`: `SessionUpSender` type mismatch
