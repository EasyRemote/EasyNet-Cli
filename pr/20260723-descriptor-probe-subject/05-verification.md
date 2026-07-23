# Verification

## Checks

- `cargo test descriptor_catalog_probe_subject --features axon-pb`
- `cargo test runtime_descriptor_remote_probe --features axon-pb`
- `cargo test descriptor_resolution_errors_project_canonical_runtime_codes --features axon-pb`
- `cargo fmt --check`
- `git diff --check`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
- `/Users/macbook.silan.tech/.local/bin/codegraph index .`
- `/Users/macbook.silan.tech/.local/bin/codegraph query "target_owned_descriptor_catalog_subject_ura" --limit 20`
- `/Users/macbook.silan.tech/.local/bin/codegraph query "DescriptorCatalogProbeSubject" --limit 20`

## Results

- `cargo test descriptor_catalog_probe_subject --features axon-pb`: passed,
  2 tests.
- `cargo test runtime_descriptor_remote_probe --features axon-pb`: passed,
  3 tests.
- `cargo test descriptor_resolution_errors_project_canonical_runtime_codes --features axon-pb`:
  passed, 1 test.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph index .`: passed, indexed
  1,018 files.
- `/Users/macbook.silan.tech/.local/bin/codegraph query "target_owned_descriptor_catalog_subject_ura" --limit 20`:
  no results.
- `/Users/macbook.silan.tech/.local/bin/codegraph query "DescriptorCatalogProbeSubject" --limit 20`:
  found the enum, variants, and `from_callee` / `into_ura` methods in
  `src/ffi/invocation/mod.rs`.

## Note

The requested codegraph override path was not present in this environment.
The installed 1.4.1 binary is available at
`/Users/macbook.silan.tech/.local/bin/codegraph` and was used for this slice.
