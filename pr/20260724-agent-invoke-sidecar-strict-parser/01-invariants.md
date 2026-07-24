# Invariants

1. Invocation metadata ownership belongs to the canonical runtime envelope, not to ability business arguments.
2. Caller URA, request ID, idempotency key, and timeout cannot be supplied through `<agent>.invoke` JSON args.
3. Unknown top-level fields fail closed, including underscore-prefixed fields.
4. The signed ability args digest covers only `args`; the parser must not carry hidden data around that can be interpreted by later layers.
5. Existing public behavior for valid `{ "ability_ura": "...", "args": {...} }` calls is preserved.
