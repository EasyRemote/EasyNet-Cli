# API Contract

`remoteapp-crash-restart-recovery-e2e.sh` passed report must include:

- `scenarios[]`
- required scenario names:
  - `daemon_restart_active_session`
  - `plugin_worker_restart`
  - `terminal_receipt_replay_after_crash`
  - `stale_socket_restart_cleanup`
- each scenario binds:
  - `selected_resource_ura`
  - `session_id`
  - `descriptor_version`
  - `events`
  - `recovery`

Scenario-specific summary fields must prove same-session daemon recovery, plugin-worker media recovery, terminal receipt replay, and automatic stale socket cleanup.

The product gate rejects missing, duplicate, unknown, failed, or semantically incomplete crash/restart summaries.
