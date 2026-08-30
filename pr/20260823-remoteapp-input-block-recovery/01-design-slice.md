# RemoteApp input runtime block recovery slice

## Product requirement

If a RemoteApp session loses input execution permission and the daemon restarts
before the operator resolves it, the recovered session must still project the
input-only blocker through `show_session`. A restart must not erase the recovery
action or make the UI infer state from stale events.

## Boundary

- The RemoteApp plugin recovery snapshot owns daemon-local session continuity.
- The snapshot stores product/session state only; Axon receipts remain separate.
- Runtime input blockers are restored only for non-terminal sessions.

## Implemented slice

- Add optional `input_runtime_block_reason` to the recovery snapshot contract.
- Derive it from the session aggregate and restore it during non-terminal
  rehydrate.
- Keep old snapshots loadable by treating the field as optional.
- Add recovery/session regressions and closure gates.

## Non-claims

- This does not prove live daemon crash/restart E2E.
- This does not prove live macOS Accessibility revoke E2E.
- This does not implement OS input injection.
