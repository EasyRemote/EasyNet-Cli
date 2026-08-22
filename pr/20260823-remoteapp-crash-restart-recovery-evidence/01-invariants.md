# RemoteApp crash/restart recovery evidence invariants

1. A live pass must use `proof_mode=real_crash_restart_recovery_matrix`.
2. `component_mock=false` and `real_backend_runtime=true` are required.
3. The artifact must keep `product_complete_claim=false`.
4. Every scenario must use public RemoteApp abilities, including
   `remote_desktop.create_session`, `remote_desktop.show_session`,
   `remote_desktop.watch_events`, and `remote_desktop.end_session`.
5. Active-session daemon restart must recover the same `session_id`, selected
   Resource URA, descriptor version, target binding epoch, and transport epoch.
6. Restart recovery must prove recovered idempotency state, replay guards, and
   lock ownership before new operations are accepted.
7. Watch-event and media paths must reattach after restart without silently
   ending the daemon session.
8. Plugin worker restart must recover or rebind target/media state without
   minting a new public session.
9. Crash during terminal receipt commit must replay the original terminal
   receipt and make repeated `end_session` idempotent.
10. Stale control/invocation socket cleanup must be explicit and followed by a
    ready daemon/runtime endpoint.
