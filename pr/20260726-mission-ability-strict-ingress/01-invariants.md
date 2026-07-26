# Invariants

1. Descriptor schema and handler runtime validation must agree.
2. Unknown mission fields are caller bugs, not forward-compat data.
3. Type mismatches never select defaults.
4. `mission.run` remains the single orchestration entry; no `easynet.*` or action alias is introduced.
5. Validation happens before mission execution, run lookup, or cancellation mutation.
