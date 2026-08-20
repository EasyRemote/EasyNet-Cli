# Verification

## Completed during implementation

- `cargo test insert_rejects_malformed_presence_key_before_mutation --features axon-pb -- --nocapture`
  - Result: passed during the first migration compile.
  - Caveat: the run started before the final unused-Result cleanup, so it is not final acceptance evidence.

## Final acceptance evidence

- `cargo test insert_rejects_malformed_presence_key_before_mutation --features axon-pb -- --nocapture`
  - Result: passed after final cleanup.
- `cargo test insert_rejects_non_principal_presence_key_before_mutation --features axon-pb -- --nocapture`
  - Result: passed after final cleanup.
- `cargo test handle_list_user_devices_filters_canonical_non_device_principals --features axon-pb -- --nocapture`
  - Result: passed after replacing malformed legacy fixture with canonical Agent principal fixtures.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - Result: passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - Result: passed.
- `cargo fmt --all`
  - Result: applied.
- `cargo fmt --check`
  - Result: passed.
- `git diff --check`
  - Result: passed.
