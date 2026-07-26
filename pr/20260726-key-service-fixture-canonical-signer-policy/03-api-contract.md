# API Contract

Input contract for fixture `sign` requests:

- `self_ura`: runtime owner URA.
- `public_key_b64`: caller-visible public projection for that owner.
- `signer_policy_ref`: canonical `provider-key-inventory:sha256:*` policy ref
  produced by `daemon::identity::signer_policy_ref(self_ura, self_ura, public_key)`.
- `canonical_bytes_b64`: descriptor-bound signing bytes.

Error contract:

- Missing or malformed fields return fixture parse/base64 errors.
- Missing owner key returns `not_found`.
- Public-key or policy mismatch returns `policy`.

The fixture must not admit the retired `daemon-key-inventory:*` namespace.
