# Decisions Log

2026-07-06:
- Treat signer-handle policy validation as DTO-boundary enforcement, not daemon
  keyring policy implementation.
- Keep local Ed25519 providers as signer providers over daemon canonical bytes;
  this does not claim daemon keyring storage cutover.
- Require `invocation.sign` usage on signer handles. Empty usage was too weak
  for a daemon-authorized signing boundary and made Go/Python policy semantics
  less explicit.
