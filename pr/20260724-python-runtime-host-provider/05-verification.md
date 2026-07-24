# Verification

## Focused tests

- `sdk/python/.venv/bin/python -m pytest sdk/python/tests/test_provider_ownership.py sdk/python/tests/test_runtime_admin.py sdk/python/tests/test_cabi.py sdk/python/tests/test_transport.py sdk/python/tests/test_runtime_environment.py -q`
- `(cd sdk/go && go test -tags runtime_cabi . -run 'TestCABIRuntimeHostStartConfigProjectsFacadeShape|TestCABIRuntimeHostStartConfigRejectsUnsupportedTransportFields|TestOpenCABIRuntimeLifecycle')`
- `cargo test parse_start_config --features axon-pb -- --nocapture`

Result: all passed. Python reported `110 passed, 4 subtests passed`; Rust FFI parser reported 3 focused tests passed.

## Codegraph

- `codegraph sync .`
- `codegraph query "providers/easynet" --limit 80`
- `codegraph query "provider/easynet" --limit 80`
- `codegraph query "providers.runtime.lifecycle" --limit 80`
- `codegraph query "providers.runtime.transport" --limit 80`

Result: Python `providers/easynet` had no results. Remaining product-named provider seam is Go `provider/easynet`.

## Gates

- `bash tools/scripts/check-sdk-product-neutrality.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`

Result: all passed.

## Formatting and diff hygiene

- `gofmt -w sdk/go/cabi_runtime.go sdk/go/cabi_runtime_test.go sdk/go/live_smoke_cabi_test.go`
- `cargo fmt --all`
- `cargo fmt --check`
- `git diff --check`

Result: all passed.

## Conformance attestation

- `python3 sdk/conformance/rebuild_public_api_model.py --write`

Result: canonical public API inventory and parity matrix now reflect Python runtime provider lifecycle/transport ownership.
