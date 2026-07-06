# Architecture

`IdentityClient.Signer` / `IdentityClient.signer` owns signer-handle decoding
because the signer handle is part of the Directory + Identity profile. Runtime
Core signing consumes that typed handle through `Signer`, but Runtime Core must
not parse daemon keyring inventory itself.

Validation belongs at the DTO boundary:
- malformed daemon projections fail immediately;
- signing providers receive only validated handles;
- prepared policy matching remains in Runtime Core signing because it compares
  signer handle policy to the prepared signing material.

The change is intentionally language-symmetric across Go and Python.
