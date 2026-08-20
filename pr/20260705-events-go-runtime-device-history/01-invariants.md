# Invariants

- The Go SDK must not invent event rows or stream cursors.
- The Invocation carrier must preserve caller, callee, subject, nonce, causal
  context, and descriptor version.
- Descriptor refs must be delegated through `IdentityClient`.
- Runtime transport may submit the daemon `events.device.history` Invocation
  and return the daemon-projected page; live stream subscriptions remain owned
  by `RuntimeClient.InvokeStream`.
- Bounded page validation remains in the Events facade.
