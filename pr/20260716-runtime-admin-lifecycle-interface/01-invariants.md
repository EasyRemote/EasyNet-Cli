Invariants
==========

1. `RuntimeAdminClient` is a lifecycle orchestrator, not a runtime-host owner.
2. Go and Python runtime admin facades both depend on neutral lifecycle
   contracts.
3. Runtime handles remain the only objects that own handle state, status
   transitions, detach state and transport calls after start/attach.
4. The admin facade may validate context and nil handles, but must not bypass
   `RuntimeHandle` lifecycle validation.
5. Public behavior remains compatible for callers that pass `*RuntimeHost` or
   the source-compatible `DaemonControl` alias.
6. No EasyNet, EasyRemote, device-directory or product process policy is added
   to the SDK admin facade.
