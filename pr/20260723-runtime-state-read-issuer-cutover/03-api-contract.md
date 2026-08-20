# API Contract

No public CLI arguments, output fields, or SDK API names change in this slice.

Internal contract:

- `LocalRuntimeStateReadIssuer::invoke` is the only local product entry for
  daemon runtime-state reads.
- `invoke_local_ability` remains available only for product actions that have
  not yet been assigned a stronger named issuer.
- New runtime-state read files must be added to
  `tools/scripts/check-runtime-state-read-subject-boundary.sh`.
