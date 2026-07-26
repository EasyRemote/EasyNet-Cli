# Local state purge boundary

## Goal

Retire the half-reset behavior where `easynet reset` only removes pairing
credentials while leaving local keyring, descriptor/read-model, registry, and
daemon discovery state available to contaminate a new canonical runtime boot.

## Non-goals

- Do not add signer provisioning fallback.
- Do not make descriptor resolution tolerate old local rows.
- Do not change Axon invocation tuple semantics.
- Do not stage unrelated `docs/spec/*` worktree changes.

## Acceptance criteria

- Local destructive reset has an explicit operator-controlled purge mode.
- Purge mode removes the EasyNet local state root instead of enumerating legacy
  compatibility files one by one.
- Running-daemon guard remains fail-closed unless `--force` is supplied.
- Existing reset behavior remains compatible when purge mode is not requested.
