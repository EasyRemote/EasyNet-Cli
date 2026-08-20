# Verification

## Commands

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

- Go, Python, Rust, Java, and Node helpers reject missing or null
  `causal_context`.
- Go, Python, Rust, Java, and Node helpers reject missing or null `args`.
- Existing retired tuple alias and unknown field rejection remains active.
- SPEC v2 rejects helper-side tuple defaulting patterns.
