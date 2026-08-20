Python static contract boundary proof

## Owner

The SDK conformance layer owns static public-model evidence. Runtime behavior
stays in SDK implementation tests and live smoke scripts.

## Boundary

- `sdk/python/easynet_sdk` owns Python runtime facade implementation.
- `sdk/python/tests` owns behavior fixtures and regression coverage.
- `sdk/conformance/python_sdk_type_contract.py` owns the strict type-level
  expectations for selected public runtime model DTOs.
- `tools/scripts/check-python-sdk-static-contract.sh` owns execution of Ruff
  and mypy over those sources.

## Rejected alternatives

- Running only Ruff is insufficient because it does not prove public DTO type
  contracts.
- Hiding the contract inside cutover readiness is insufficient because the gate
  needs an independently callable owner for local development and CI.
- Running mypy over every source file in this slice would expand the refactor
  beyond the existing contract and risk turning one enforcement slice into a
  broad annotation migration.
