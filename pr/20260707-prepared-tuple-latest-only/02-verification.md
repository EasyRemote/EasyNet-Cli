# Verification

## Focused SDK Checks

- `cd sdk/go && go test . -run 'TestPreparedInvocationRejectsMissingCanonicalBytes|TestPreparedInvocationRejectsMaterialFieldsInTuple|TestPreparedInvocationDecodesCurrentABIShape|TestCompatibilityProjectsModelsChatStreamAndFiles|TestGoMEMCExecutesSharedProfileExclusivityConformanceCase' -count=1`
  - Result: pass.
- `PYTHONPATH=sdk/python python3 -m pytest -q sdk/python/tests/test_signing.py::SigningTests::test_prepared_invocation_rejects_missing_canonical_bytes sdk/python/tests/test_signing.py::SigningTests::test_prepared_invocation_rejects_material_fields_in_tuple sdk/python/tests/test_signing.py::SigningTests::test_prepared_invocation_decodes_current_abi_shape sdk/python/tests/test_compatibility.py`
  - Result: 7 passed.

## Aggregate Audit

- `bash tools/scripts/check-sdk-completion-audit.sh`
  - Result: `SDK completion audit ok`.
  - Included passing SDK scaffold, parity matrix, conformance reports, section 27 coverage, FFI ABI v4 header, URA naming, daemon latest input boundary, backend SDK-only boundary, product smokes, Python SDK live smoke, and Go SDK live smoke.
