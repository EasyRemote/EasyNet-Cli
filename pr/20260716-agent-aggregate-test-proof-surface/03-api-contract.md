# API Contract

## Public Behavior

No public CLI, daemon, FFI, or SDK behavior changes.

## Internal Boundary

- Production remains free to call repository-owned loaders and snapshot methods
  used by daemon runtime paths.
- Test-only snapshot helpers are not part of the production internal contract.
- The convergence script continues to prove that the aggregate owns the
  relevant concepts; compile output proves proof-only helpers are absent from
  production builds.

## Compatibility

No compatibility path or fallback is added.
