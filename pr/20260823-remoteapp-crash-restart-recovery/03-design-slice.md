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
- the same recovered `session_id` may start a new media generation when the
  client performs fresh signaling;
- watch-events can replay a `SESSION_REHYDRATED` or degraded/retry event;
- `show_session` returns the same `session_id` instead of `session_not_found`;
- `end_session` remains idempotent and can close the recovered row.

Stage 2 still needs live watch/media transport reattachment evidence and must
satisfy the stricter `daemon_restart_active_session` verifier scenario.

## Acceptance evidence

- Unit tests for snapshot round-trip and fail-closed corrupted snapshot loading.
- Unit tests for startup batch isolation: one corrupt snapshot must be reported
  and skipped without dropping valid recoverable rows.
- Unix permission checks for daemon-local recovery state: the store directory is
  private and snapshot files are private because snapshots include the
  daemon-local session token required for post-restart control access.
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
- Wired the store into the plugin runtime through
  `RemoteDesktopPlugin::persist_recovery_snapshot`, keeping persistence owned by
  the remote-desktop plugin instead of Axon, frontend state, or a generic daemon
  session registry.
- Added `RemoteDesktopRecoverySnapshot::from_session` so durable state is
  derived from the canonical `RemoteDesktopSession` aggregate rather than from
  ad hoc handler projections.
- Persisted recovery snapshots at the current session mutation write
  boundaries:
  - `remote_desktop.create_session`;
  - `remote_desktop.refresh_lease`;
  - `remote_desktop.show_session` after audit reads that may expire a session;
  - `remote_desktop.end_session`;
  - lease watchdog timeout expiry;
  - create-session preflight/insert pruning of already-expired sessions.
- Added unit coverage for:
  - valid snapshot round-trip;
  - corrupt snapshot fail-closed behavior;
  - corrupt-row isolation during startup batch recovery;
  - private Unix recovery directory/file permissions for session-token
    snapshots;
  - path-unsafe session id rejection;
  - selected Resource URA validation.
- Added handler regression coverage for:
  - `create_session` writing a non-terminal recovery snapshot;
  - `end_session` overwriting that snapshot with terminal receipt and bounded
    event-log projection.
- Added Stage 1 startup rehydration:
  - daemon startup loads recovery snapshots back into `RemoteDesktopSessionStore`;
  - non-terminal snapshots rehydrate as degraded/suspended sessions with
    `SESSION_REHYDRATED`;
  - recovered rows preserve `session_id`, selected Resource URA, caller,
    consent receipt, session token, target binding, event replay, and terminal
    receipt;
  - corrupt or mismatched snapshot rows are reported and skipped instead of
    aborting the whole startup recovery batch;
  - recovered non-terminal sessions can leave the rehydrated `Suspended` phase
    and start a new media negotiation epoch without minting a replacement
    session id;
  - public `show_session`, `watch_events`, and `end_session` operate through the
    normal session access/lifecycle path after rehydration.

This slice is intentionally not marked as full product recovery. Stage 1
control-plane rehydration exists and the lifecycle can accept a fresh media
epoch, but media/input transports are deliberately not marked ready merely
because a snapshot was loaded. The live crash/restart verifier remains a
product blocker until actual media reattachment, frame rendering after restart,
cross-process evidence, and endpoint cleanup all pass.
