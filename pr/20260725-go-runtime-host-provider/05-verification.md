# Verification

## Codegraph

- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph query "provider/easynet" --limit 80`

Result: synced changed files; no results found for `provider/easynet`.

## Focused and package tests

- `cd sdk/go && go test ./provider/runtime ./provider/runtime/pluginexec`
- `cd sdk/go && go test ./...`

Result: all Go SDK packages passed.

## Conformance attestation

- `python3 sdk/conformance/rebuild_public_api_model.py --write`

Result: canonical public API inventory now classifies `sdk/go/provider/runtime`
as provider-neutral core and no longer lists `sdk/go/provider/easynet`.

## Gates

- `bash tools/scripts/check-sdk-product-neutrality.sh --self-test`
- `bash tools/scripts/check-sdk-product-neutrality.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`

Result: all passed.

## Formatting and diff hygiene

- `gofmt -w sdk/go/provider/runtime/lifecycle.go sdk/go/provider/runtime/lifecycle_test.go`
- `cargo fmt --check`
- `python3 -m py_compile sdk/conformance/rebuild_public_api_model.py sdk/conformance/sdk_concepts.py`
- `git diff --check`

Result: all passed.
