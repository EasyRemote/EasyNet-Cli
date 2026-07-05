# Invariants

- `mission.events` descriptor refs are produced by the shared daemon SDK
  carrier builder, not by Go/Python string assembly.
- Go/Python C ABI transports call `easynet_invocation_invoke` for events; a
  direct projection-only path is not a production execution implementation.
- Projection accepts daemon runtime result wrappers without dropping the request
  cursor or mission id context.
- Cursor and limit validation remains bounded: non-negative cursor, non-negative
  limit, and `limit <= 1000`.
- No compatibility fallback or legacy NotImplemented branch remains for
  `mission.events` in C ABI Mission transports.
