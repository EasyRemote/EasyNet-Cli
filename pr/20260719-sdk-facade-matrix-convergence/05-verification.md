# Verification

## Planned

- CodeGraph structure query for plugin template generation and helper packages.
- Focused template/helper tests.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`.
- `bash tools/scripts/check-sdk-cutover-readiness.sh`.

## Results

- CodeGraph status: index current, 994 files, 33,786 nodes, 128,948 edges.
- CodeGraph explored `plugin init language sidecar template` and
  `plugin_exec sidecar helper provider easynet`; blast radius centered on
  `src/cli/commands/groups/plugin_template.rs`,
  `sdk/python/easynet_sdk/providers/easynet/plugin_exec.py`, and
  `sdk/go/provider/easynet/pluginexec/pluginexec.go`.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`: passed.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`: passed.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -p easynet cli::commands::groups::plugin_template::tests --lib`: passed, 6 tests.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt`: passed.
- `bash tools/scripts/check-sdk-cutover-readiness.sh`: passed.
