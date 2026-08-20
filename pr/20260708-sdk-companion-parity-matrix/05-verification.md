# Verification

- `tools/scripts/check-sdk-parity-matrix.sh --self-test`
  - Passed.
- `tools/scripts/check-sdk-parity-matrix.sh`
  - Passed.
- `go test . -run 'TestGoSDKExecutesSharedParityMatrixConformanceCase|TestDaemonHandleExposesDesktopCompanionLifecycle|TestCABIRuntimeCompanion'`
  - Passed from `sdk/go`.
- `PYTHONPATH=sdk/python python3 -m pytest -q sdk/python/tests/test_conformance.py::SharedConformanceFixtureTests::test_python_sdk_executes_shared_parity_matrix_conformance_case sdk/python/tests/test_daemon.py sdk/python/tests/test_cabi.py -k companion`
  - Passed: 3 tests.
- `git diff --check`
  - Passed.
- `rg -n "\b[U]R[I]\b|\bu[r]i\b" sdk/conformance/cases/runtime-companion-control.yaml sdk/conformance/sdk-parity-matrix.json sdk/conformance/cases/sdk-go-python-parity-matrix.yaml tools/scripts/check-sdk-parity-matrix.sh sdk/go/conformance_test.go sdk/python/tests/test_conformance.py sdk/conformance/runner/go-action-adapter-report.json sdk/conformance/runner/python-action-adapter-report.json pr/20260708-sdk-companion-parity-matrix -g '!target'`
  - Passed: no matches.
