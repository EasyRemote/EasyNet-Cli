# SDK Provider Evidence Rebind

## Intent

Restore the SDK conformance evidence chain for stale Go and Python
action-adapter report entries.

## Expected effect

- **Effect type:** architecture convergence.
- **Root fork addressed:** SDK/provider proof evidence drift.
- **Concrete use case:** `check-sdk-conformance-reports.sh` validates that every
  action-adapter report is backed by current repository-local evidence before
  executing the adapter selector. Several Go/Python report entries point at
  stale hashes, so the SDK provider evidence chain is not auditable.

## Non-goals

- Do not promote any matrix cell from `seam` to `provider-backed`.
- Do not change SDK runtime behavior.
- Do not edit access-control tests in this slice.
