# Intent — RemoteApp Session Timeout E2E

RemoteApp already has lease timeout unit coverage, but product readiness needs
host-level evidence that a public session created through the CLI reaches a
deterministic terminal state after its lease expires.

This change adds a host E2E harness for the timeout path:

1. Select a live display/window/application Resource URA through
   `resource.refresh_remote_targets`.
2. Create a short-lived `remote_desktop.create_session` with the Resource URA
   as Invocation subject.
3. Wait past the lease deadline.
4. Re-read the session through `remote_desktop.show_session`.
5. Require a closed session with `terminal_receipt.reason_code=session_expired`.
6. Invoke `remote_desktop.end_session` afterward and require idempotent audit
   state instead of a second terminal fact.

This improves timeout lifecycle evidence. It does not prove reconnect, crash
recovery, cross-device network behavior, or successful interactive input.
