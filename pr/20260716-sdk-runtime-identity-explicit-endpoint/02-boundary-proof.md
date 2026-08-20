Runtime identity boundary proof

## Owner

Runtime identity custody belongs to the daemon key-service. The SDK owns only a
narrow signing facade over explicit endpoint transport.

## Boundary

- Product/runtime lifecycle owns endpoint discovery and process placement.
- Daemon key-service owns identity existence, key generation, public-key
  projection, and signing.
- SDK owns request validation, projection shape, signature verification against
  the projected public key, and typed error projection.

## Deleted path

`DefaultRuntimeIdentitySocketPath` and `default_runtime_keyring_socket_path`
had no provider-backed implementation and always returned `INVALID_ARGUMENT`.
Keeping either as a canonical public API preserved a legacy discovery state
without a lifecycle owner. Removing both makes the only available path explicit
endpoint injection.

## Preserved behavior

The usable runtime identity APIs remain:

- `LoadRuntimeSigningIdentity`
- `EnsureRuntimeSigningIdentity`
- `RuntimeSigningIdentity.Sign`
- `RuntimeSigningIdentity.SignCanonical`
- `RuntimeSigningIdentity.SigningPublicKey`
- `load_runtime_signing_identity`
- `ensure_runtime_signing_identity`
- `RuntimeSigningIdentity.sign_canonical`

Empty endpoint rejection remains covered through load and ensure requests.
