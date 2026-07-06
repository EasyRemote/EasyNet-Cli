# Invariants

- Invocation dispatch remains complete-tuple based: caller URA, callee URA, descriptor ref, subject URA, nonce, causal context, and args stay inspectable before dispatch.
- Compatibility profile naming must remain profile-owned and must not become an external-protocol canonical daemon model.
- Public aliases that only preserve older input names are architectural debt and must be removed.
- Private implementation bridges may keep transport symbol names required by C ABI or daemon carriers, but those names must not define public SDK inputs.
- Go and Python must converge on the same logical DTO names.
