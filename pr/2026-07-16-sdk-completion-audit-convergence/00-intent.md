# SDK Completion Audit Convergence

## Intent

Make the SDK completion audit an executable part of SDK cutover readiness without
creating a shell recursion between `check-sdk-cutover-readiness.sh` and
`check-sdk-completion-audit.sh`.

## Expected effect

- **Effect type:** architecture convergence.
- **Root fork addressed:** SDK capability state ownership between the generated
  parity matrix and the completion audit gate.
- **Concrete use case:** a capability cell can be a valid `seam` because it has
  live execution evidence even when that language has no public SDK shape
  evidence. The audit must accept that generated state, while still rejecting
  empty seam claims and provider-backed cells without provider proof.

## Non-goals

- Do not add new SDK product features.
- Do not weaken provider-backed proof requirements.
- Do not introduce compatibility fallback paths.
