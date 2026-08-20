# Invariants

1. `RuntimeSigningIdentity` never manages User URAs.
2. `RuntimeCallerCustody` classifies User URAs as managed-user custody.
3. Remote invocation caller signer loading goes through
   `load_runtime_caller_signer`.
4. Remote invocation caller signer loading must not call
   `RuntimeSigningIdentity::load_default`.
5. Missing User signer errors must not contain runtime-owner keyring lookup
   wording such as `keyring entry not found`.
