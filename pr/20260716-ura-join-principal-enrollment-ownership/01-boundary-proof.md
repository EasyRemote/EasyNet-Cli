# Boundary Proof

## Source of Truth

`PrincipalEnrollmentProof` remains the federation join input. The CLI may read
its `principal_ura` before sending the proof to `federation.join`, but the
federation client still owns the actual proof transmission.

## Public Behavior

- URA joins with user principal proofs persist `username` and `user_id` from the
  user principal URA.
- URA joins without a user principal proof keep those fields empty.
- The federation join request still receives the original optional proof.

## Non-Goals

- No new credential fields.
- No compatibility fallback.
- No daemon or Axon protocol changes.
