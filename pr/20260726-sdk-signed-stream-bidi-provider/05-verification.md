# Verification

Passed:
- `go test .` from `sdk/go`.
- `PYTHONPATH=... sdk/python/.venv/bin/python -m unittest sdk/python/tests/test_runtime.py sdk/python/tests/test_authorized_runtime_session.py`.
- `PYTHON=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python/.venv/bin/python bash tools/scripts/check-canonical-runtime-convergence-v2.sh`.
- `PYTHON=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python/.venv/bin/python bash tools/scripts/check-sdk-canonical-public-api.sh`.
- `bash tools/scripts/check-architecture-convergence.sh`.
- `cargo fmt --check && git diff --check`.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync . && /Users/macbook.silan.tech/.local/bin/codegraph status .`.

Generated evidence:
- `sdk/conformance/canonical-public-api.json`.
- `sdk/conformance/sdk-parity-matrix.json`.
