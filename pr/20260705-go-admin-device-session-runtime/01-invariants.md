# Invariants

- Device-session create/delete must lower to complete daemon Invocations with
  caller, callee, descriptor ref, subject, nonce, causal context, and args.
- Descriptor refs must be resolved through `IdentityClient`; Go must not build
  `ability@version` strings.
- Runtime dispatch must use `RuntimeClient.Invoke`; product calls must not use
  control frames.
- The Go facade may project daemon output, but daemon remains the authority for
  session id, state, route, expiry, and deletion outcome.
- Delete accepts only daemon device-session ids; browser/product session ids
  remain rejected.
