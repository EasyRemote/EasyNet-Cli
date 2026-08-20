Intent
======

Remove the non-canonical `presented_pubkey_hex` request shape from
`federation.resolve_key`.

The canonical runtime already constructs presented-key pins as base64 through
`ResolveKeyRequest::with_presented_pubkey_b64`. Keeping an inbound hex repair
path in the daemon handler creates a second proof material representation at
the trust/admission boundary.
