# RemoteApp session cancel E2E intent

Date: 2026-08-23

## Problem

RemoteApp product lifecycle evidence covered lease timeout and idempotent
post-timeout `end_session`, but user-initiated cancel/close still lacked a
host-level executable proof. That left the frontend `End session` product flow
dependent on handler/unit evidence instead of a public CLI/daemon path.

## Intent

Add a host-side RemoteApp session cancel E2E harness that:

- selects a live display/window/application Resource URA through
  `resource.refresh_remote_targets`;
- creates a RemoteApp session through the public CLI helper for
  `remote_desktop.create_session`;
- invokes public `remote_desktop.end_session` with product reason
  `user_cancelled`;
- observes the closed session through public `remote_desktop.show_session`;
- invokes `remote_desktop.end_session` again and proves the result is
  idempotent while preserving the original terminal receipt.

## Non-goals

- Do not redefine RemoteApp cancel as Axon transport-level `invocation.cancel`.
- Do not claim reconnect, crash recovery, consent revoke, input injection,
  cross-device media, or network fallback product completion.
- Do not move RemoteDesktop plugin behavior into runtime core.
