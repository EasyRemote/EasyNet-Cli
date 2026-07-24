# Intent

## Goal

Refactor the Python SDK consumer boundary auditor so raw local runtime-host
access is modeled with runtime-host terminology instead of daemon terminology.

## Non-goals

- Do not weaken detection of raw control sockets, runtime-host sockets, raw FFI,
  or runtime-host subprocess spawning.
- Do not keep old `raw_daemon_session` diagnostic aliases.
- Do not change direct runtime provider behavior.

## Acceptance criteria

- `consumer_boundary.py` no longer defines daemon-named audit helpers.
- Raw runtime-host socket/session violations report `raw_runtime_host_session`.
- Product runtime binaries remain forbidden subprocess targets.
- Focused consumer boundary tests and SPEC gates pass.
