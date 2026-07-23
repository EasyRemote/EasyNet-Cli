# Architecture

`PreparedInvocation` is the SDK boundary object for canonical runtime signing.
It sits between provider-backed prepare transport and caller-side signing.

The removed fallback was architecturally wrong because it let the SDK become a
payload repair layer. That makes provider conformance weaker and allows missing
prepare facts to survive until later admission or receipt validation. The SDK
must instead validate the canonical object it receives.

Ownership:

- Provider/runtime owns producing explicit prepare facts.
- SDK owns validating canonical prepare object shape.
- Signing material owns canonical bytes and signer policy.
- No product-specific provider owns the canonical SDK model.
