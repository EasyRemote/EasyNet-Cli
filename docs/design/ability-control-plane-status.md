# Ability Control-Plane -- Implementation Status

**Status:** verified-against-disk snapshot.
**Date:** 2026-06-23.
**Scope:** Current state of the three-registry ability control plane in
`EasyNet-Cli`, including the Axon `LocalRuntime` receipt proof-binding seam.

This note supersedes `docs/spec/ability-catalog-internal-split-v1.md`. That
older plan deliberately avoided the three registries and should now be treated
as archival context only.

---

## 1. Landed Model

The control-plane model is implemented under `src/runtime/ability/`:

| Concept | Lives in | State |
|---|---|---|
| `AbilityDescriptorRegistry` | `ability/descriptor.rs` | built + unit-tested |
| `AuthorityBindingRegistry` | `ability/authority.rs` | built + unit-tested |
| `AbilityImplRegistry` | `ability/impl_binding.rs` | built + unit-tested |
| `AbilityControlPlaneRegistry` | `ability/registry.rs` | built + unit-tested |

The three truths are now first-class:

- descriptor truth: ability name, descriptor version, call mode, schema hash
- authority truth: advertise/invoke authority scope
- implementation truth: implementation source, runtime environment, impl hash

`AbilityImplSource` covers `NativeDaemon`, `BuiltinPlugin`, `SidecarPlugin`,
`DeclarativePlugin`, `Eal`, and `Mcp`.

## 2. Catalog Wiring

`AxonAbilityCatalog` owns an `AbilityControlPlaneRegistry`.

Every registration path writes a control-plane record alongside the executable
handler. Dynamic catalogue drain removes the record on hot-unregister, so the
control-plane registry does not leak stale plugin/MCP bindings.

The plugin host consumes this boundary through
`rebind_control_plane_record(...)`: a plugin supplies an implementation binding;
it does not become the authority owner of the ability.

Representative canaries:

- `register_rpc_writes_control_plane_record`
- `runtime_registration_carries_control_plane_proof_binding`
- `control_plane_keeps_rpc_and_stream_records_for_same_ability`
- `control_plane_audit_facts_bind_descriptor_authority_and_impl`

## 3. Axon Receipt Proof Binding

The previous status note said CLI could not inject receipt proof facts because
Axon lacked an affordance. That is no longer current.

Current flow:

1. `AxonAbilityCatalog::runtime_options_for(...)` builds per-mode
   `AbilityOptions`.
2. `bind_runtime_proof_for_mode(...)` reads the control-plane record and calls
   `AbilityOptions::with_mode_proof_binding(...)` with an
   `AbilityProofBinding`.
3. Axon `LocalRuntime` normalizes that proof binding into receipt proof facts
   for descriptor-bound invocations.
4. Terminal receipts carry signed descriptor/runtime proof facts:
   descriptor version, schema hash, impl hash, subject/input/output hashes, and
   related proof slots.

The receipt shape and canonical signed bytes remain Axon-owned. CLI supplies the
binding facts through Axon's runtime options; it does not construct or fork
receipt bodies.

## 4. Remaining Work

The remaining gap is product/query visibility, not receipt construction:

- Unary `InvokeResponse` should expose the Axon admission and terminal receipts
  when the daemon routed execution through `LocalRuntime`.
- The seven-axes e2e suite should assert non-default descriptor/runtime proof
  facts on a real daemon invocation.
- Ledger/watch may later project selected proof facts for audit UX, but those
  projections must remain copies of signed receipt facts.

## 5. Anti-redo Note

Do not rebuild the three registries and do not return to the old split-v1
"owner map + manifest store only" plan. The fuller descriptor/authority/impl
model is the current implementation direction.

Do not revive standalone `policy` or `trust-level` product surfaces to solve
permission work. Future permission work belongs under a unified ability
access/permission model.
