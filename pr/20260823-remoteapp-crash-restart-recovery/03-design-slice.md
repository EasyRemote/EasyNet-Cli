# Design Slice

## Target architecture

Add a daemon-local RemoteApp session recovery store owned by the
remote-desktop plugin:

```text
RemoteDesktopSession
  -> bounded durable session snapshot
  -> daemon-local recovery store
  -> plugin startup rehydration
  -> public show_session/watch_events/end_session recovery behavior
```

The store is not an Axon receipt store and not a frontend DB table. It is a
plugin-owned projection that lets the plugin recover its own lifecycle state
after process restart while preserving Axon's public Invocation/receipt
boundary.

## Minimal durable facts

- schema version
- session id
- creator caller URA
- selected Resource URA
- target binding snapshot
- consent approval receipt reference
- consent grant scope
- requested mode
- transport preferences
- video constraints
- input policy
- created/updated/lease expiry timestamps
- lifecycle state
- terminal receipt, if terminal
- bounded event log cursor/events

## Rehydration policy

Stage 1 should not pretend media survived a daemon crash. On restart:

- non-terminal sessions are restored as recoverable session rows;
- media/input transports are marked not ready;
- watch-events can replay a `SESSION_REHYDRATED` or degraded/retry event;
- `show_session` returns the same `session_id` instead of `session_not_found`;
- `end_session` remains idempotent and can close the recovered row.

Stage 2 can reattach watch/media transport and satisfy the stricter
`daemon_restart_active_session` verifier scenario.

## Acceptance evidence

- Unit tests for snapshot round-trip and fail-closed corrupted snapshot loading.
- A live runner that kills/restarts the daemon and verifies same-session public
  `show_session` recovery.
- Later, a full `remoteapp-crash-restart-recovery-e2e.sh --run --runner-cmd ...`
  pass covering all verifier scenarios.

## Implemented in this slice

- Added `plugins/remote-desktop/src/session_recovery.rs`.
- Added `RemoteDesktopRecoverySnapshot` as a schema-versioned durable session
  projection contract.
- Added `RemoteDesktopRecoveryStore` with atomic snapshot save/load and
  path-safe session id validation.
- Added unit coverage for:
  - valid snapshot round-trip;
  - corrupt snapshot fail-closed behavior;
  - path-unsafe session id rejection;
  - selected Resource URA validation.

This slice is intentionally not marked as product recovery. The store still
needs to be wired to session mutation boundaries and plugin startup rehydration.
