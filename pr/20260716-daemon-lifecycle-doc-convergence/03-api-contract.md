# API Contract

No public API changes in this slice.

Documentation describes these existing contracts:

- Product invoke/stream/bidi: daemon Invocation surface over `daemon.sock`.
- Control: boot/status subscriptions and cancellation only.
- Lifecycle projection: `runtime.json` carries `runtime_kind = DaemonOnly` as
  metadata, not process authority.
- Status source: `JoinConnectionSnapshot` and daemon discovery facts.

Retired bridge/runtime-dispatch states are invalid current lifecycle inputs, not
compatibility modes.
