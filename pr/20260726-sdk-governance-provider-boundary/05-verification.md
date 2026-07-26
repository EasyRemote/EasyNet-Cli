# Verification

- `PYTHONPATH="sdk/python:sdk/python/tests" "$SDK_CONFORMANCE_PYTHON" -m unittest sdk/python/tests/test_ability_invocation.py sdk/python/tests/test_runtime_ability.py`
- `(cd sdk/go && go test ./...)`
- `cargo fmt --check`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph query "runtime governance descriptor provider AbilityInvocationClient reject governance read" --path .`

Result: all verification passed.
