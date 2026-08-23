# Intent

Prevent an executing Device from resolving its own realm Authority key over
the same session that is currently delivering an Authority-signed invocation.

The live EasyNet catalog read proved that a same-realm Authority caller was
classified as external. Device dispatch then waited for
`federation.resolve_key` on the blocked carrier and failed after ten seconds.

Acceptance criteria:

- A same-realm Authority with the exact presented key in the local trust anchor
  is admitted without network resolution.
- A missing or mismatched same-realm Authority key fails closed locally.
- Cross-realm Authority keys continue to use ephemeral Hub attestation.
- A live Hub-to-Device catalogue read no longer reports
  `CALLER_KEY_NOT_FOUND` or a `resolve_key` timeout.
