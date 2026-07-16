# Verification

Passed:

```text
cargo test --lib stream_chunk_json_decodes_json_payload
cd sdk/go && go test -tags easynet_direct_runtime ./...
cd sdk/python && PYTHONPATH=.:../EasyNet-Axon/sdk/python:tests .venv/bin/python -m unittest discover -s tests
python3 sdk/conformance/rebuild_public_api_model.py --write
python3 sdk/conformance/sdk_concepts.py --validate-schema
python3 sdk/conformance/sdk_concepts.py --validate-actual
bash tools/scripts/check-sdk-product-neutrality.sh
bash tools/scripts/go-sdk-live-smoke.sh
bash tools/scripts/python-sdk-live-smoke.sh
cargo fmt --check
gofmt -d sdk/go/direct_runtime.go sdk/go/direct_runtime_test.go sdk/go/stream_test.go sdk/go/conformance_test.go sdk/go/directory_test.go sdk/go/cabi_runtime_test.go
git diff --check
```

Both live-smoke commands pass. They invoke the same daemon through the C ABI,
then assert that the first server-stream event is `data` through the public Go
and Python facades.

`bash tools/scripts/check-sdk-parity-matrix.sh` still reports
`live_results_required`. Its live-result attestation is an external CI input;
it is distinct from the two successful local smoke runs and is not forged in
the repository.
