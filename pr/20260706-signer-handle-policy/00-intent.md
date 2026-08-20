# Intent

## Goal
Strengthen the Directory + Identity signer-handle projection used by Runtime Core
prepare/sign/submit so Go and Python facades treat a signer handle as a
daemon-authorized policy object, not a loose field bag.

## Non-Goals
- Do not implement daemon keyring storage policy.
- Do not add product-specific signer concepts.
- Do not change Invocation canonical material or signature algorithms.
- Do not modify `docs/spec/daemon-sdk-requirements-v1.md`.

## Acceptance Criteria
- Go and Python reject signer handles without the SDK profile, Ed25519
  algorithm, local-daemon signing mode, invocation signing usage, matching policy
  signer id, and valid public-key metadata when present.
- Existing public constructors remain source compatible.
- Shared conformance records that signer handles are daemon-policy-bound.
- Runtime signing tests continue to prove that prepared material becomes a
  SignedInvocation before submit.
