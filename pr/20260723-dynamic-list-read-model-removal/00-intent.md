# Dynamic Ability List Read Model Removal

## Goal

Remove the unused `list_dynamic_abilities` read model from the daemon ability catalogue so dynamic execution rows cannot be interpreted as a publication or discovery source.

## Non-goals

- Do not change public ability invocation behavior.
- Do not introduce product-specific EasyNet/EasyRemote SDK abstractions.
- Do not preserve compatibility for dead internal diagnostics.

## Acceptance criteria

- `AxonAbilityCatalog` keeps dynamic diagnostics on explicit predicates such as `has_dynamic`.
- Public ability discovery remains owned by committed control-plane records and exact call-mode projections.
- Convergence gates reject reintroduction of dynamic/static union publication wording or helpers.
