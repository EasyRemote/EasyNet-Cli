# Python runtime authority binding

## Invariants

- Local SDK ability facades must preserve the complete Invocation tuple and attach authority metadata only after descriptor/subject projection is finalized.
- User callers may self-mint exact-scope delegation only for same-user subjects.
- Device and SystemAgent subjects remain daemon-owned admission cases and must not be rewritten into user delegation.
- Key custody remains behind the daemon runtime key service; SDK callers receive only metadata-bound authority proof.

## Implementation plan

1. Add a local runtime authority provider that binds already-finalized `InvocationDraft` values.
2. Inject that provider into environment-created ability facades so the default SDK path does not omit `x-runtime-delegation`.
3. Keep low-level clients explicit: tests and advanced users may still construct clients without an authority provider.
4. Verify direct provider behavior, `RuntimeAbilityClient`, and generic `AbilityInvocationClient` draft construction.

## Verification

- `python3 -m py_compile sdk/python/easynet_sdk/environment.py sdk/python/easynet_sdk/runtime_authority.py sdk/python/easynet_sdk/runtime_ability.py sdk/python/easynet_sdk/ability_invocation.py sdk/python/tests/test_runtime_authority.py sdk/python/tests/test_runtime_ability.py sdk/python/tests/test_ability_invocation.py`
- `uv run pytest -q sdk/python/tests/test_runtime_authority.py sdk/python/tests/test_runtime_ability.py sdk/python/tests/test_ability_invocation.py sdk/python/tests/test_runtime_environment.py`
