# Verification

Checks completed:

- `go test . -run 'TestRuntimeReceipt|TestInvocationResultSeparatesAdmissionAndTerminalReceipts|TestRuntimeClientInvokeReturnsTypedResult|TestRuntimeAbilityClientDispatchesProviderLifecycleSurfaces|TestRuntimeAbilityChildContextDispatchesWithParentReceiptCausality'`
- `PYTHONPATH=sdk/python /opt/anaconda3/bin/python -m pytest sdk/python/tests/test_runtime.py sdk/python/tests/test_runtime_ability.py sdk/python/tests/test_ability_invocation.py -k 'receipt or invocation_result_separates or runtime_ability or child_context or provider_lifecycle'`
- `/opt/anaconda3/bin/python -m black --check sdk/python/easynet_sdk/runtime.py sdk/python/tests/test_runtime.py`
- `PYTHON_BIN=/opt/anaconda3/bin/python bash tools/scripts/check-sdk-canonical-public-api.sh`
- `PYTHON_BIN=/opt/anaconda3/bin/python bash tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `PYTHON_BIN=/opt/anaconda3/bin/python bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `PYTHON_BIN=/opt/anaconda3/bin/python bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
- `cargo fmt --all -- --check`
