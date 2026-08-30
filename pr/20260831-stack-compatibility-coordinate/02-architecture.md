# Architecture

## Ownership

- `compatibility/axon.lock.json`: sole pinned Axon coordinate.
- `tools/scripts/check-axon-lock.py`: coordinate parser, workflow output, and checked-out-contract verifier.
- Existing Rust/Python package metadata gates: owning dependency-resolution proof.
- `.github/workflows/tests.yml`: pinned semantic admission.
- Candidate workflow: explicit upstream-candidate validation without lock mutation.
- Runtime/Python release workflows: artifact verification with local sources disabled.

## State progression

`AxonCandidateReady → CliCompatible → ProductCompatible → MergeEligible → Released`

Only deterministic successful evidence advances the state. A failed suite returns the candidate to `AxonCandidateReady`; no downstream manifest can self-attest advancement.
