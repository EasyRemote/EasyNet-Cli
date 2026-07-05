# API Contract

## Request

`AbilityImplID`

- `impl_id`: required non-empty string.
- `ability_ura`: required Ability URA.
- `metadata`: optional object carried through request metadata.

## Daemon System Abilities

- Enable: `ability.impl.enable`
- Disable: `ability.impl.disable`

Both carriers must preserve the caller-provided profile tuple where applicable and expose the complete Invocation before dispatch.

## Result

`PublicationRecord`

- Enable kind: `ability_impl_enabled`
- Disable kind: `ability_impl_disabled`
- Status: `enabled` or `disabled`
- Metadata includes source ability, `ability_ura`, `impl_id`, raw daemon result, and request metadata.

## Errors

- Missing fields: `INVALID_ARGUMENT`
- Non-Ability URA: `INVALID_ARGUMENT`
- Daemon invocation failure: `ABILITY_FAILED`
- Transport/C ABI failure: typed SDK transport error
