# API Contract

Public behavior retained:

- Device starts still attempt `session.open` against the configured hub.
- Session errors continue to be reported through existing supervisor/error
  paths.

Internal contract:

- No pre-session REST call may mutate or repair hub trust state.
- Trust publication/repair belongs to signed prelude invocations.
- If a hub lacks required trust state, the session attempt fails through the
  canonical gRPC status path.
