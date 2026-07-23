# Invariants

1. Product-state reads use `LocalRuntimeStateReadIssuer`.
2. Product mutations remain on the action invocation path.
3. `skill.list` public flags and output are unchanged.
4. `api-key list` public flags and output are unchanged.
5. Boundary gates reject regression of these reads to `invoke_local_ability`.
