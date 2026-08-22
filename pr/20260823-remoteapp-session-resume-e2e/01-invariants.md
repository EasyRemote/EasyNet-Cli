# RemoteApp session resume E2E invariants

## Invocation and subject invariants

- The selected target remains the Invocation `subject` Resource URA.
- `create_session` args must not carry `subject`, `subject_ura`, or
  `resource_ura`.
- `refresh_lease`, `show_session`, and `end_session` must all use the same
  selected Resource URA and session id.
- Session token remains descriptor-contract data; it must not be projected by
  `show_session`.

## Lifecycle invariants

- `refresh_lease` must extend `lease_expires_at_ms` beyond the original created
  lease.
- The resume validation must run after the original lease would have expired.
- `show_session` after the original lease must return the same non-terminal
  session and the refreshed lease.
- Cleanup `end_session` must produce a terminal receipt with
  `reason_code=resume_e2e_cleanup`.

## Evidence boundaries

- This proves the daemon/session half of short disconnect resume.
- It does not prove browser WebRTC restart, long-outage reconnect,
  crash/restart recovery, NAT/relay fallback, cross-device resume, or input
  control continuity.
