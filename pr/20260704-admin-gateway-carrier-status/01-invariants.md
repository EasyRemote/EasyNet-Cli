# Invariants

1. Admin operations are daemon product lifecycle helpers, not backend account
   operations.
2. Every mutation carrier must preserve the complete Invocation tuple.
3. `GatewayStatus.ready` may be true only when daemon process, control,
   Invocation runtime, directory, and trust readiness requirements are satisfied.
4. Control liveness and Invocation readiness are distinct states.
5. Public listener readiness is an observable field, not assumed from local UDS
   readiness.
6. Trust readiness must come from daemon/control/product-presence facts, not
   from browser auth or backend database rows.
7. Agent lifecycle records model daemon registry rows and lifecycle ability
   outcomes; they are not EasyRemote Python objects.
8. Missing optional agent URAs remain null/absent; the SDK must not fabricate
   hosted agent identities.
9. Pairing/session CRUD gaps must stay visible as partial coverage, not hidden
   behind compatibility fallback.
10. Gateway status facades may hold a daemon lifecycle handle, but readiness
    classification still belongs to the Rust Admin + Gateway projection, not to
    language-specific Go or Python branches.
