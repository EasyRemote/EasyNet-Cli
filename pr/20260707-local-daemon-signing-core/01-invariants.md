# Invariants

- `PreparedInvocation` remains non-submit-ready.
- `SignedInvocation` is the only submit-ready pre-runtime object.
- Signing material canonical bytes remain daemon/Axon-owned.
- A signed object must carry a non-empty signer id, either from explicit
  `SignerPolicy.signer_id` or from the attached signature key hint.
- Signed objects must preserve the prepare-time signer policy exactly; later
  submission must not reconstruct or infer that proof.
- Local-daemon signing must be added as a daemon/keyring-backed transition, not
  as a language-facade compatibility fallback.
