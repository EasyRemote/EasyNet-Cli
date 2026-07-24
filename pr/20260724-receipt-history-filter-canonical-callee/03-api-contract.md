# API Contract

## Accepted receipt-history filter fields

- `caller_ura`
- `callee_ura`
- `subject_ura`
- `subject_uras`
- `ability_ura`
- `ability_uras`
- `state`
- `trace_id`

## Rejected fields

- `agent_ura` is rejected at the daemon wire boundary.

## CLI facade

If the CLI still accepts an `--agent-ura` option for operator familiarity, it must lower to `filter.callee_ura` before invoking the runtime ability.

## Errors

Unsupported fields return a deterministic parse error before ledger access.
