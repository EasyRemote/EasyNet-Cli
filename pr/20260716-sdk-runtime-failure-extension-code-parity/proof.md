# SDK Runtime Failure Extension Code Parity

## Intent

Align the Python SDK runtime failure projection with the Go SDK boundary:
runtime/daemon failure responses may carry canonical domain extension codes
outside the SDK enum, while shared daemon error DTO decoding remains strict.

## CodeGraph Evidence

- `sdk/go/errors.go` exposes `runtimeFailureCode`, which first accepts enum
  schema codes via `ParseErrorCode`, then preserves uppercase domain extension
  codes, and rejects retired aliases such as `DAEMON_DOWN`.
- `sdk/go/errors_test.go` proves `AXON_MEMBERSHIP_REQUIRED` and
  `TARGET_NOT_IN_PRESENCE_REGISTRY` are preserved, while `InvalidArgument` and
  `DAEMON_DOWN` collapse to `PROTOCOL_MISMATCH`.
- `sdk/python/easynet_sdk/errors.py` currently routes
  `canonical_failure_code` through `normalize_error_code`, which rejects every
  code outside the Python `ErrorCode` enum and therefore loses valid domain
  failure specificity.
- `sdk/python/easynet_sdk/transport.py` and
  `sdk/python/easynet_sdk/direct_runtime.py` use `canonical_failure_code` only
  for runtime result/frame response projection, not for strict daemon error DTO
  decoding.

## Boundary Decision

Keep `SDKError.from_json` and `normalize_error_code` strict for
`sdk/schemas/error.schema.json`. Broaden only the runtime failure projection so
extension codes remain visible to callers as exact machine strings. Legacy or
mixed-case aliases still project to `PROTOCOL_MISMATCH`.

## Verification Plan

- `go test ./... -run 'TestRuntimeFailureCodePreservesDomainCodesAndRejectsLegacyAliases|TestParseErrorCodeAcceptsOnlyCanonicalSchemaValues'`
- `PYTHONPATH=sdk/python:sdk/python/tests python -m pytest sdk/python/tests/test_errors.py -q`
- `PYTHONPATH=sdk/python:sdk/python/tests python -m pytest sdk/python/tests/test_transport.py -k 'runtime_failure or envelope_errors' -q`
- `bash tools/scripts/check-architecture-convergence.sh`
- `git diff --check`

## Verification Results

- PASS: `go test ./... -run 'TestRuntimeFailureCodePreservesDomainCodesAndRejectsLegacyAliases|TestParseErrorCodeAcceptsOnlyCanonicalSchemaValues'`
- PASS: `PYTHONPATH=sdk/python:sdk/python/tests python -m pytest sdk/python/tests/test_errors.py -q`
- PASS: `PYTHONPATH=sdk/python:sdk/python/tests python -m pytest sdk/python/tests/test_errors.py sdk/python/tests/test_transport.py -k 'canonical_failure_code or extension_failure_codes or non_ok_runtime_result or extension_failure_code or envelope_errors' -q`
- PASS: `PYTHONPATH=sdk/python:sdk/python/tests python -m pytest sdk/python/tests/test_errors.py sdk/python/tests/test_transport.py -q`
- PASS: `bash tools/scripts/check-architecture-convergence.sh`
- PASS: `bash tests/scripts/test_check_architecture_convergence.sh`
- PASS: `git diff --check`
