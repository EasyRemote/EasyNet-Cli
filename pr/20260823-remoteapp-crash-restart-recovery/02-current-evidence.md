# Current Evidence

## Live probe

Artifact directory:

`target/e2e/remoteapp-crash-restart-probe/20260822-223509-45956`

Probe sequence:

1. Start a sentinel window fixture.
2. Create a public RemoteApp window session with a long lease.
3. Kill the daemon process with `kill -9`.
4. Start the daemon again with `easynet runtime start`.
5. Call `remote_desktop.show_session` using the original create-session JSON.

Observed result:

```text
remote_desktop.show_session: session "rd-crash-probe-45956" not found; reason=session_not_found
```

Runtime status after restart showed a new daemon PID and healthy control/
invocation sockets, so the failure is not a startup failure. The session itself
was not rehydrated.

## Root cause

`RemoteDesktopPlugin::with_target_binding_verifier` creates a fresh
`RemoteDesktopSessionStore::new()` on every daemon process start.

`RemoteDesktopSessionStore` is an in-memory `HashMap<String,
RemoteDesktopSession>`.

`RemoteDesktopEventLog` is an in-memory bounded ring.

There is no durable session snapshot, terminal receipt replay store, or startup
rehydration path for RemoteApp sessions.

## Product status

Crash/restart recovery remains incomplete. The existing verifier correctly
requires real recovery scenarios and must not be relaxed to accept this failure.

