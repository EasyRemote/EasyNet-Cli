# Invariants

- A daemon crash must not erase the public auditability of an already-created
  RemoteApp session.
- Recovery state belongs to the remote-desktop plugin inside the EasyNet daemon.
  Axon still owns Invocation/receipt/admission semantics; the plugin owns
  RemoteApp session lifecycle state.
- A live recovery artifact must be produced by real daemon/plugin process
  restart behavior. Source-only tests, self-tests, and synthetic verifier rows
  are not product evidence.
- The verifier must remain strict: required scenarios are
  `daemon_restart_active_session`, `plugin_worker_restart`,
  `terminal_receipt_replay_after_crash`, and `stale_socket_restart_cleanup`.
- Durable state must not persist bearer tokens in frontend-visible docs or
  reports. If a token must be persisted for idempotent session-control
  recovery, it must remain daemon-local and never appear in public views.
- Recovery implementation must be explicit about degraded behavior. If active
  media cannot be reattached yet, the session should fail closed into a visible
  terminal/recovery state rather than returning `session_not_found`.

