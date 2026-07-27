Invariants
==========

- `federation.resolve_key` request DTO exposes one presented-key field:
  `presented_pubkey_b64`.
- The daemon must not decode or repair `presented_pubkey_hex`.
- Unknown `presented_pubkey_hex` input must fail closed through
  `serde(deny_unknown_fields)`.
- Multi-key user disambiguation remains descriptor-bound and byte-exact after
  decoding the base64 pin.
- Response compatibility is outside this slice; existing response parser gates
  already reject legacy response repair paths.
