# API Contract

## SignerHandle

Required fields:
- `profile = "directory_identity"`
- `signer_id`
- `owner_ura`
- `key_id`
- `algorithm = "ed25519"`
- `policy`
- `metadata`

Required policy facts:
- `mode = "local_daemon_signing"`
- `usage = "invocation.sign"`
- optional `signer_id`, when present, must match top-level `signer_id`

Metadata:
- may include `public_key_base64`;
- `public_key_base64`, when present, must be a valid 32-byte Ed25519 public key;
- private key material is rejected by existing identity request guards.

Errors:
- invalid signer handle projections map to InvalidArgument / invalid profile
  payload, matching existing SDK error behavior.
