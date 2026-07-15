# Verification

## Commands

```text
bash -n tools/scripts/check-architecture-convergence.sh
tests/scripts/test_check_architecture_convergence.sh
tools/scripts/check-architecture-convergence.sh
(cd sdk/go && go test ./...)
PYTHONPATH=sdk/python python3 -m pytest sdk/python/tests/test_cabi.py -q
git diff --check
```

## Result

All commands passed.

## Notes

The self-test includes a negative fixture that restores the old terminal
stream/bidi cancel contract and terminal SDK projections, and expects R20 to
reject them.
