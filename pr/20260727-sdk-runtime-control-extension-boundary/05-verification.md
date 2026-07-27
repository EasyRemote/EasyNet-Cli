Verification results:
- `go test ./...` in `sdk/go`: passed.
- `PYTHONPATH="$PWD/sdk/python:$PWD/../EasyNet-Axon/sdk/python" python -m pytest sdk/python/tests/test_control_ipc.py sdk/python/tests/test_environment.py -q`: passed, 22 tests.
- `bash tools/scripts/check-sdk-product-neutrality.sh`: passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`: passed; index up to date.

Notes:
- A first Python test attempt with ambient system `PYTHONPATH=sdk/python` failed during collection because `axon_sdk` was not importable. The repository test path requires `../EasyNet-Axon/sdk/python`; rerun with that canonical path passed.
