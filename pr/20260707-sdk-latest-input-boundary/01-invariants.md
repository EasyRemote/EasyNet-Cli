# Invariants

- Invocation dispatch remains complete-tuple based: caller URA, callee URA, descriptor ref, subject URA, nonce, causal context, and args stay inspectable before dispatch.
- Compatibility profile naming must remain profile-owned and must not become an external-protocol canonical daemon model.
- Public aliases that only preserve older input names are architectural debt and must be removed.
- Private implementation bridges may keep transport symbol names required by C ABI or daemon carriers, but those names must not define public SDK inputs.
- Go and Python must converge on the same logical DTO names.
- Runtime Core prepare options must serialize only the current daemon/C ABI
  fields: `expires_in_ms`, `signer_id`, `policy_ref`, and
  `local_daemon_signing`.
- Legacy prepare-option inputs such as descriptor resolution, nonce filling,
  and required-user-signature flags are not SDK aliases; if needed later they
  must return through the SPEC as canonical policy fields.
