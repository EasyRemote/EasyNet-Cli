# API contract

Go:
- `NewPrincipalClient(provider PrincipalLifecycle)` constructs the client.
- `PrincipalClient` implements `PrincipalLifecycle`.
- `RuntimePrincipalProvider` implements `PrincipalLifecycle`.

Python:
- `PrincipalLifecycle` is the provider protocol.
- `PrincipalClient` accepts `PrincipalLifecycle`.
- `RuntimePrincipalProvider` implements the protocol structurally.

Errors:
- Existing validation messages for missing principal lifecycle dependency remain semantically equivalent and product-neutral.
