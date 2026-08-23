# RemoteApp cross-device hard gate

## Product seam

The cross-device product smoke already reported whether child E2Es observed
distinct caller/provider device URAs, but a completed run could still be marked
`passed` while `local_provider_boundary_only=true`. That is too weak for the
product objective: local-provider or same-device evidence must not satisfy a
cross-device regression gate.

## Slice

- Downgrade a nominally passed report to failed when no distinct device URAs are
  observed.
- Require the failure reason to explicitly state that distinct device URAs were
  not observed.
- Preserve the existing non-claims: this gate still does not prove real OS
  capture, input injection, host audio, NAT/TURN deployment, or frontend
  rendering.

## Expected impact

The smoke gate now distinguishes "the child scripts ran" from "cross-device
topology was proven". It prevents local-only topology evidence from being
counted as cross-device product progress.
