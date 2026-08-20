# Verification

## Commands

```text
bash -n tools/scripts/check-architecture-convergence.sh
tests/scripts/test_check_architecture_convergence.sh
tools/scripts/check-architecture-convergence.sh
(cd sdk/go && go test ./...)
(cd sdk/go && go test -tags easynet_direct_runtime -run 'TestDirectRuntime(Stream|Bidi)CancelProjectsNonTerminalRequest' .)
PYTHONPATH=sdk/python python3 -m pytest sdk/python/tests/test_cabi.py -q
PYTHONPATH=sdk/python python3 -m pytest sdk/python/tests/test_stream.py sdk/python/tests/test_bidi.py -q
PYTHONPATH=sdk/python python3 -m pytest sdk/python/tests/test_direct_runtime.py -q
git diff --check
```

## Result

All commands passed.

## Notes

The self-test includes negative fixtures that restore the old terminal
stream/bidi cancel contract, terminal provider projections, and direct SDK
facades that accept terminal local cancel outcomes. R20 must reject each fork.

## Delta

- Tightened Go/Python direct stream and bidi cancel facades so `cancel()` only
  accepts `CancelRequested` with `terminal=false`.
- Updated direct SDK tests from terminal cancellation to cancel-request
  projection.
- Extended R20 to cover C ABI providers, direct runtime transports, direct SDK
  facades, and deterministic rejection tests.
- Added direct runtime behavior tests for Go and Python stream/bidi cancel
  projection.
