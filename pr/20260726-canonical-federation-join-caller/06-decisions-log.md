Decisions
=========

- 2026-07-26: Treat the non-URA federation-join pseudo-caller as a legacy
  identity model. Keep bounded candidate-key leasing, but move proof semantics
  out of caller naming and into bootstrap admission.
- 2026-07-26: Bootstrap proof accepts `public_key_hex` as the request-scoped
  candidate key. Signature/key equality is enforced by descriptor-bound Axon
  admission through `BootstrapCandidateKeyProvider`; the proof layer enforces
  tuple, realm, route, payload, and key-shape facts.
