# Provider-managed signing custody convergence

## Goal

Remove the executable SDK/runtime leak named `local_daemon_signing` from the canonical signing model and converge it into a product-neutral runtime concept: `provider_managed_signing`.

The existing flow is legitimate: a prepared invocation may be signed by a provider-owned custody service before submission. The architectural defect is the public mode name and DTO state exposing "local daemon" as the canonical SDK concept.

## Root abstraction problem

The SDK defines the canonical runtime model. It must not name custody by the current product implementation (`local daemon`). A daemon may implement the provider, and the C ABI may still expose a native entrypoint that calls the local process boundary, but the public policy state must describe the capability generically.

## Boundary invariants

1. Public SDK/conformance/schema policy mode is `provider_managed_signing`.
2. No public SDK parser accepts `local_daemon_signing` as a compatibility alias.
3. The Rust signing state machine has distinct states for caller signing and provider-managed signing.
4. Go and Python SDKs detect provider-managed signing with equivalent logic.
5. Signer handles remain provider-bound: signing profile, invocation signing usage, `provider_key_inventory` provenance source, policy ref, active key state, and owner binding are still required.
6. Native C ABI symbol names may remain `runtime_invocation_sign_prepared_local` for binary boundary stability, but the serialized signed invocation policy must not expose the local-daemon name.
7. Conformance evidence names and case IDs must be product-neutral.

## Verification plan

- Target Rust invocation/CABI tests covering prepared local/provider signing.
- Go signing and CABI tests.
- Python signing, managed signing, and CABI tests.
- SDK public API model rebuild.
- Canonical runtime convergence v2 gate.
- Architecture convergence gate.
- `cargo fmt --check`.

## Decisions

- Use `provider_managed_signing` as the canonical mode because it describes the SDK state without naming EasyNet, EasyRemote, daemon, localhost, or keyring implementation.
- Use `provider_key_inventory` and `provider-key-inventory:sha256` for canonical signer provenance/policy references. Daemon/keyring names are implementation details and are not accepted as SDK compatibility aliases.
- Do not keep `local_daemon_signing` fallback parsing. Existing manifests or fixtures using the old mode must migrate.
