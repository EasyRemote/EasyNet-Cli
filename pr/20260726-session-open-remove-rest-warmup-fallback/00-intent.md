# Intent

Goal: remove the REST credential warmup backstop that runs before each
`session.open` dial.

Non-goals:

- Do not add another retry/fallback path.
- Do not change the public `session.open` ability name or wire contract.
- Do not weaken signed gRPC prelude/session admission.

Acceptance criteria:

- `session.open` initiation no longer calls `/api/v1/devices/verify-credential`.
- There is no `CredentialWarmupOutcome` state machine beside the canonical
  session phase/prelude state machine.
- Existing session/prelude tests continue to pass through signed gRPC paths.
- A convergence gate prevents the REST warmup module/path from returning.
