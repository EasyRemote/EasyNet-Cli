# Verification

Completed.

Commands:

- `bash -n tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
- `cargo fmt --check`
- `git diff --check`
- `cd sdk/python && uv run pytest tests/test_import_boundary.py tests/test_environment.py tests/test_cabi.py`
- `git ls-files sdk/python/easynet_sdk | grep -E '(^|/)(__pycache__/.*|[^/]+\.pyc$)' || true`
- `find sdk/python/easynet_sdk sdk/python/tests -path '*/__pycache__/*' -type f | wc -l`
- `/Users/macbook.silan.tech/.local/bin/codegraph index .`
- `/Users/macbook.silan.tech/.local/bin/codegraph query "EXPECTED_ABI_VERSION" --limit 20`
- `/Users/macbook.silan.tech/.local/bin/codegraph query "generic_v5" --limit 20`

Results:

- SPEC v2 self-test passes.
- SPEC v2 main gate passes.
- Legacy architecture convergence gate passes.
- Formatting and diff whitespace checks pass.
- Python SDK targeted tests pass: 45 passed.
- No tracked Python SDK bytecode remains in `sdk/python/easynet_sdk`.
- Local ignored Python bytecode artifacts are removed after test execution.
- Codegraph finds the canonical C ABI version constant and the Python SDK tests
  referencing it.
- Codegraph reports no `generic_v5` symbol results.
