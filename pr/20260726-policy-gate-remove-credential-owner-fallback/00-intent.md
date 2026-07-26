# Intent

## Goal

Remove the ordinary admission-policy dependency on local `credentials.json` owner projection. Runtime policy admission must resolve accountable device ownership from canonical trust facts, descriptor/URA semantics, or explicit verified authority only.

## Non-goals

- Do not remove the bounded bootstrap authority verifier that uses paired-device credentials during first trust publication.
- Do not change the public invocation API or the descriptor-bound Axon tuple.
- Do not introduce a compatibility fallback for missing trust-owner rows.

## Acceptance criteria

- `AdmissionPolicyGate` no longer calls `local_device_owner_fact`.
- Device principals without a canonical trust-owner fact enter policy as devices with no caller owner.
- Device/callee owner resolution no longer reads local credentials as a fallback.
- Tests prove saved or malformed local credentials cannot affect ordinary policy owner resolution.
- Architecture/SPEC gates continue to pass.
