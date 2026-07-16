# API Contract

## Public CLI

Existing `--actor-ura` flags remain optional where they are optional today.
When omitted, the CLI uses subject-self authorization for the same commands as
before.

## Internal CLI Contract

`principal_command` accepts `PrincipalCommandActor`, `idempotency_key`,
`expected_version`, `proof_kind`, and `proof_ref`. It does not inspect
`principal_ura` or choose an actor fallback.

## Daemon Contract

PrincipalLifecycle ability payloads continue to carry:

- `request.command.actor_ura`;
- `request.command.idempotency_key`;
- `request.command.proof.kind`;
- `request.command.proof.reference`;
- optional `request.command.expected_version`.

The daemon remains fail-closed for malformed or non-canonical actor URAs.
