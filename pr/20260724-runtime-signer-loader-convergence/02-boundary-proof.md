# Boundary Proof

`daemon::identity::self_identity` owns the signer custody state machine. It knows whether a caller is a managed User or a runtime owner.

`daemon::boot::invocation::identity` owns daemon identity derivation from credentials but not key-service custody selection.

`cli::commands::federation_wire` owns local trust-file update orchestration but not signer loading policy.

Therefore both upper-layer callers must depend on `load_runtime_caller_signer`, not `RuntimeSigningIdentity::load_default`.

