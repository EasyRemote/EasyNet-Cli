# Verification

## Commands

- `cargo test sidecar_invocation_envelope --features axon-pb`
- `cargo test sidecar_ --features axon-pb`
- `go test ./provider/easynet/pluginexec` from `sdk/go`
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python python -m pytest /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python/tests/test_plugin_exec.py`
- `cargo test --manifest-path sdk/rust/provider/easynet/pluginexec/Cargo.toml`
- `node --test sdk/node/test/pluginexec.test.mjs`
- `mvn -q -Dtest=run.runtime.sdk.provider.easynet.pluginexec.SidecarRuntimeTest test` from `sdk/java`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `cargo fmt --check`
- `git diff --check`

## Result

All commands passed.

## Evidence

- Daemon sidecar frame decode rejects missing `causal_context`.
- Daemon sidecar frame decode rejects missing `args`.
- Daemon sidecar frame decode rejects null `causal_context`.
- Daemon sidecar frame decode rejects null `args`.
- Sidecar/pluginexec fixtures use canonical `{"form":"none"}` causal context.
- SPEC v2 rejects daemon sidecar `serde(default)` tuple repair and non-canonical
  causal-context fixtures.
