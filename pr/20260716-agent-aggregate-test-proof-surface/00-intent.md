# Intent

## Slice

Confine Agent aggregate snapshot proof helpers to test builds.

## Root Fork

The daemon-owned Agent aggregate read model has two surfaces:

- production repository loaders, used by governance, skill, dispatch, and
  resource paths;
- snapshot convenience methods and projection fields that are only consumed by
  unit tests.

Keeping test-only convenience readers in production compilation makes the
aggregate boundary look wider than the real runtime contract and leaves dead
code warnings in the production library build.

## Expected Effect

- Architecture convergence: production callers continue to use the repository
  owner instead of broad snapshot helpers.
- Architecture cleanliness: test proof helpers remain available only to tests.
- Product acceleration: warning-free compile evidence is easier to audit in the
  SDK cutover gates.
