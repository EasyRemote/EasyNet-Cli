# Intent

## Goal

Remove the receipt-history `agent_ura` compatibility filter from canonical SDK/runtime request shapes and converge all SDKs on product-neutral invocation tuple predicates.

## Non-goals

- Do not remove real directory-event `agent_ura` facts where the domain object is an advertised Agent.
- Do not rename public CLI flags in this iteration; existing CLI input may remain as a facade spelling only if it lowers to canonical runtime fields.
- Do not add a second daemon-side receipt filter model or fallback parser.

## Acceptance criteria

- `invocation.history.*` daemon ability filters accept `callee_ura`, `subject_ura`, `subject_uras`, `caller_ura`, `ability_ura`, `ability_uras`, `state`, and `trace_id`, but no `agent_ura`.
- CLI `--agent-ura` compatibility input, if still present, serializes only to canonical `filter.callee_ura`.
- Node SDK receipt filter no longer exposes or serializes `agent_ura`.
- Go/Python/Node receipt filter model remains aligned on canonical fields.
- Tests prove `agent_ura` is rejected at daemon wire boundary and absent from SDK serialization.
