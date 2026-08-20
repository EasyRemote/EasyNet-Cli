Execution Checklist
===================

- Remove `presented_pubkey_hex` from `ResolveKeyRequest`.
- Remove hex-to-base64 repair from `handle_resolve_key`.
- Update tests to assert the retired field fails closed.
- Extend SPEC v2 gate to reject the retired field and conversion logic.
- Run focused resolve-key tests and convergence gates.
