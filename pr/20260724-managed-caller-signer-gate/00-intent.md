# Intent

Prevent remote invocation caller signing from regressing to runtime-owner
keyring lookup for User callers.

The canonical signer custody split is explicit: daemon runtime owners use
runtime-owner signing identity; User callers use managed, subject-bound signing
identity. A missing User managed key must fail in the managed-user custody model,
not fall back to `derive_pubkey(user_ura)`.
