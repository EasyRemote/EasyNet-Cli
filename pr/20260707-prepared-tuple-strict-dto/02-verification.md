# Verification

All commands were run from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`
on 2026-07-07.

- `cd sdk/go && go test . -run 'TestPreparedInvocation|TestCompatibilityProjectsModelsChatStreamAndFiles|TestGoMEMCExecutesSharedProfileExclusivityConformanceCase' -count=1`
  - Result: pass.
- `PYTHONPATH=sdk/python python3 -m pytest -q sdk/python/tests/test_signing.py sdk/python/tests/test_conformance.py::SharedConformanceFixtureTests::test_python_memc_executes_shared_profile_exclusivity_conformance_case`
  - Result: pass, `20 passed`.
- `cd sdk/go && go test ./...`
  - Result: pass.
- `PYTHONPATH=sdk/python python3 -m pytest -q sdk/python/tests/test_signing.py`
  - Result: pass, `19 passed`.
- `bash tools/scripts/check-sdk-completion-audit.sh`
  - Result: pass.
