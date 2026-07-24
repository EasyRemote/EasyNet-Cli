# Verification

## Helper tests

- `go test ./provider/runtime/pluginexec` from `sdk/go`
- `cargo test --manifest-path sdk/rust/provider/runtime/pluginexec/Cargo.toml -- --nocapture`
- `sdk/python/.venv/bin/python -m pytest sdk/python/tests/test_plugin_exec.py sdk/python/tests/test_provider_ownership.py -q`
- `node --test sdk/node/test/pluginexec.test.mjs`
- `mvn -q -f sdk/java/pom.xml test -Dtest=run.runtime.sdk.provider.runtime.pluginexec.SidecarRuntimeTest`

Result: all passed.

## Template tests

- `cargo test init_hello_plugin_generates_go_compiled_project --features axon-pb -- --nocapture`
- `cargo test init_hello_plugin_generates_rust_compiled_project --features axon-pb -- --nocapture`
- `cargo test init_hello_plugin_generates_java_compiled_project --features axon-pb -- --nocapture`
- `cargo test init_hello_plugin_generates_node_project --features axon-pb -- --nocapture`

Result: all passed.

## Gates

- `bash tools/scripts/check-sdk-product-neutrality.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`

Result: all passed.

## Formatting and diff hygiene

- `gofmt -w sdk/go/provider/runtime/pluginexec/pluginexec.go sdk/go/provider/runtime/pluginexec/pluginexec_test.go`
- `cargo fmt --all`
- `cargo fmt --check`
- `git diff --check`

Result: all passed.

## Conformance attestation

- `python3 sdk/conformance/rebuild_public_api_model.py --write`

Result: canonical public API inventory now records the Go runtime pluginexec provider path and Python runtime provider package.
