# RemoteApp session resume E2E intent

Date: 2026-08-23

## Problem

RemoteApp had timeout, cancel, and permission-revoke lifecycle evidence, but
the short disconnect/resume path still needed host-level proof that a public
client can keep the same daemon session alive by refreshing the lease and later
validate that same non-terminal session through `remote_desktop.show_session`.

## Intent

Add a host-side RemoteApp session resume E2E harness that:

- selects a live Resource URA through `resource.refresh_remote_targets`;
- creates a session with a short initial lease through the public CLI;
- invokes public `remote_desktop.refresh_lease` with the same session token;
- waits past the original lease expiry;
- proves public `remote_desktop.show_session` still returns the same
  non-terminal session with the refreshed lease;
- closes the session through public `remote_desktop.end_session` for cleanup.

## Non-goals

- Do not claim browser WebRTC media rebind, long-outage reconnect, crash
  recovery, or cross-device resume completion.
- Do not create a new session as a substitute for resume.
- Do not move RemoteDesktop session policy into runtime core.
