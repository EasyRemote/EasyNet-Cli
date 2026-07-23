# Architecture

Root abstraction problem:

`invoke_local_ability` names a transport primitive, not an authority model. When
read-only runtime-state commands use it, they inherit the daemon-self subject
shortcut and admission failures become product-visible as subject mismatch,
missing signer, or hidden descriptor failures.

Refactoring:

- Keep `LocalRuntimeStateReadIssuer` as the single read-projection authority.
- Move remaining runtime-state read callers to that issuer.
- Extend the architecture gate to name every read-projection source file that
  must not call `invoke_local_ability`.

This preserves the single shared runtime model: the SDK/daemon still see one
canonical invocation tuple, but product read surfaces no longer default tuple
subjects at the transport boundary.
