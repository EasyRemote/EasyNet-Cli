# Verification

Checks completed:

- `go test . -run 'TestRuntimeClientRestartRecoveryProviderContract|TestRuntimeAbilityClientDispatchesProviderLifecycleSurfaces|TestRuntimeAbilityClientBuildsCompleteCanonicalDraft|TestRuntimeAbilityClientInvokesObjectResult'`
- `PYTHONPATH=sdk/python /opt/anaconda3/bin/python -m pytest sdk/python/tests/test_runtime.py sdk/python/tests/test_runtime_ability.py`
- `/opt/anaconda3/bin/python -m black --check sdk/python/easynet_sdk/runtime.py sdk/python/easynet_sdk/runtime_ability.py sdk/python/easynet_sdk/__init__.py sdk/python/tests/test_runtime.py sdk/python/tests/test_runtime_ability.py`
- `PYTHON_BIN=/opt/anaconda3/bin/python bash tools/scripts/check-sdk-canonical-public-api.sh`
- `PYTHON_BIN=/opt/anaconda3/bin/python bash tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `PYTHON_BIN=/opt/anaconda3/bin/python bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `PYTHON_BIN=/opt/anaconda3/bin/python bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
- `cargo fmt --all -- --check`
