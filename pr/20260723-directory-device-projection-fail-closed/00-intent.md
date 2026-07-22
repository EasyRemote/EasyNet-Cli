# Directory Device Projection Fail-Closed

## Goal

Remove the federation directory adapter fallback that converted non-canonical presence URAs into directory rows by using the raw URA as `node_id`.

## Non-goals

- Do not remove the boot/status control socket.
- Do not change the public Invocation transport.
- Do not introduce SDK product abstractions.

## Acceptance criteria

- Presence-to-directory snapshot and event adapters require canonical Device URAs.
- Remote directory view application rejects invalid directory frames before mutating state.
- Tests pin rejection of legacy agent-shaped and malformed presence rows.
- Convergence gates reject reintroduction of raw-URA node-id fallback wording or tests.
