Principal route manifest convergence

Goal

Collapse the duplicated Go/Python `principal.lifecycle.*` provider route
literals into one explicit EasyNet provider manifest with generated language
bindings. The public Principal client/provider APIs stay unchanged; only the
internal provider lowering source of truth changes.

Follow-up convergence in this slice moves the provider route manifest out of
`sdk/` entirely and generates Rust CLI bindings from the same manifest. That
keeps EasyNet-specific route facts in a provider-owned boundary while the SDK
continues to expose only generic runtime concepts.

Expected effect

- Architecture convergence: one provider-owned route table feeds both SDK
  languages plus the Rust CLI consumer.
- Code cleanliness: no parallel handwritten principal lifecycle ability lists
  in SDK or CLI lowering code.
- Product acceleration: future route changes are made once and regenerated.
