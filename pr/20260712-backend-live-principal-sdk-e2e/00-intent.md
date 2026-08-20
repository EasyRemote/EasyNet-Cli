Goal: close the next Backend-present PrincipalLifecycle evidence gap by proving
that Backend account signing-key registration can run through the public Go SDK
against a live daemon runtime.

This slice is not allowed to add a Backend-owned daemon lifecycle, key-service,
trust store or principal store. Backend remains an HTTP/PostgreSQL account
adapter. The daemon and SDK remain the canonical runtime owner.
