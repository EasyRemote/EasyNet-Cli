Decisions
=========

- Treat `presented_pubkey_b64` as the only canonical request-side presented key
  representation.
- Do not keep a hex compatibility layer because canonical proof material should
  not have multiple accepted encodings at admission/federation boundaries.
