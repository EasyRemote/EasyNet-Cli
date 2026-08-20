# Architecture

Before this change, `dial_and_run_session_with_idle_timeout` executed a
blocking REST credential verification task before connecting the gRPC session.
That made session admission depend on two protocols:

- REST `/api/v1/devices/verify-credential`, advisory and best-effort.
- signed gRPC prelude + `session.open`, canonical and auditable.

The canonical runtime model should have one authority path. This task removes
the REST pre-dial path so boot/session state is owned by the signed gRPC
session phase machine.
