# Remoteapp Rebind Deadline Intent

## Goal

Complete the targeted-session lifecycle seam where a remote application session can enter `Rebinding` with a projected deadline but no deterministic timeout transition when the observer produces no later target event.

## Root problem

`Rebinding` is a lifecycle phase, not a diagnostic label. A state machine that publishes `rebind_deadline_ms` must also own the deadline transition. Leaving expiry to incidental future observations makes session behavior unbounded and weakens receipt/event evidence for frontend recovery.

## Scope

- Keep remote desktop execution device-native; do not move host-local capture/input execution behind a user service.
- Add deadline expiry to the remoteapp target state machine and session store boundary.
- Let the target monitor tick enforce deadline expiry even when the platform observer emits no observation.
- Preserve public event names and frontend actions.
- Remove duplicated lifecycle evidence keys encountered in the same state-machine area.

## Non-goals

- No new service callee model for remote desktop execution.
- No fallback display capture when an application/window binding cannot be maintained.
- No change to the public ability descriptor shape unless the SPEC requires it.
