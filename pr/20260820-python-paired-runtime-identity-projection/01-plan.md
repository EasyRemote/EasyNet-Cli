# Python paired runtime identity projection

## Invariants

- SDK callers may read public paired-principal facts, but credential secrets must not escape the SDK boundary.
- Runtime host identity and paired user principal identity remain distinct.
- Control discovery and credential projection must agree on realm and runtime instance before the SDK returns a paired identity projection.
- Exception objects must preserve normal Python traceback lifecycle.

## Implementation plan

1. Add `read_paired_runtime_identity_projection` for credential-backed public identity projection.
2. Expose the projection through `SdkEnvironment` while validating the attached runtime control identity.
3. Extend `RuntimeIdentityProjection` with display-name support without exposing credential tokens.
4. Keep the Python package version and lockfile aligned with the SDK API addition.

## Verification

- `uv run pytest -q sdk/python/tests/test_runtime_environment.py sdk/python/tests/test_errors.py`
