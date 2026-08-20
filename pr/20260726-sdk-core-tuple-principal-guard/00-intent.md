Goal
====

Move all-zero principal rejection into the canonical SDK Invocation tuple
boundary for Go and Python, matching the Java/Swift behavior already present.

Non-goals
=========

- Do not add product-specific caller, device, or user policy.
- Do not parse or route URAs in the core tuple builder.
- Do not change public builder method names.

Acceptance criteria
===================

- Go `InvocationBuilder` rejects all-zero caller, callee, and subject values
  before constructing an `InvocationDraft`.
- Python `InvocationBuilder` rejects the same values before constructing an
  `InvocationDraft`.
- Existing public builder API remains source-compatible.
- The rejection happens at the SDK core tuple boundary, not in a product/provider
  adapter.
