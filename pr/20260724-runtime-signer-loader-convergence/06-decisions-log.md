# Decisions Log

- Decision: direct `RuntimeSigningIdentity::load_default` calls are only acceptable inside identity/provider implementation layers, not boot/product orchestration.
- Decision: trust auto-wire receives an owner-bound signer capability and projects its public key; it does not inspect key-service inventory directly.

