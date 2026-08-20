# Stream Error Classification

## Objective

Classify host-propagated stream payload errors with `kind: INTERNAL` as ability
execution failures instead of admission failures. This keeps product facades
from reporting a remote function exception as a permission/admission denial.

## Boundary Proof

- The SDK does not interpret application payloads except for the existing
  daemon stream error envelope shape.
- The change only maps an already-recognized remote error kind into the SDK
  taxonomy; it does not define new Axon or daemon protocol semantics.
- EasyRemote receives the SDK `ABILITY_FAILED` code and projects it into its
  public `InternalError` taxonomy.

## Validation

- `sdk/python/tests/test_transport.py`
- `sdk/python/tests/test_import_boundary.py`
- EasyRemote focused stream/client tests against the local SDK
- `ruff check sdk/python`
- `git diff --check`
