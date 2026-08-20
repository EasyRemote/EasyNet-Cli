# Invariants

1. Carrier construction must preserve the full Invocation tuple supplied by the
   caller: caller, callee, subject, nonce, causal context, descriptor version,
   args, content type, and metadata.
2. `session.create` args must only contain validated device session fields:
   `device_ura`, `hub_ura`, `session_kind`, and optional positive
   `expires_unix_ms`.
3. `session.delete` args must only contain `session_id` and optional reason;
   delete projection must not reuse a `session.list` row as mutation truth.
4. Create projection must validate the resulting SDK `DeviceSession` shape and
   may use the request only as carrier context for fields the daemon output
   intentionally omits.
5. C ABI bindings must delegate semantics to Rust contract functions and keep
   pointer/string ownership unchanged.
6. Hub lifecycle, pairing, and credential verification remain explicit gaps
   until their daemon-owned contracts exist.
