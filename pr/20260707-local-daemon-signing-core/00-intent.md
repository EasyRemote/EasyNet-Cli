# Intent

Goal: converge the Runtime Core signing state machine on the SPEC requirement
that every `Prepared -> Signed` transition records signer id and policy proof.

Non-goals:
- Do not move signing canonicalization out of Axon/daemon-owned helpers.
- Do not add product-specific EasyNet or EasyRemote signer concepts.
- Do not introduce legacy input aliases or facade-side fallback signing paths.

Acceptance criteria:
- Rust daemon SDK core `SignedInvocation` carries signer policy proof.
- C ABI signed invocation JSON projects that policy proof.
- Existing caller-signing behavior remains compatible while exposing stronger
  state evidence.
