# Ledger route callee authority convergence

## Goal

Remove the ledger route ability URA fallback that re-derived route ownership
from the invocation caller after callee ownership resolution failed.

## Root abstraction problem

Invocation route identity is part of the canonical invocation tuple. The route
ability is owned by the callee or by the explicit descriptor ref carried on the
wire. `runtime_factory.rs` still let the ledger sink try the caller as a second
publisher when callee derivation failed. That creates a second route authority
inside the persistence projection and can make read models disagree with the
route that was actually invoked.

## Architectural decision

The ledger route projection is callee-authoritative:

- explicit descriptor refs must carry an ability owned by the callee;
- bare/runtime ability names project only through the callee owner;
- caller identity is not a route ownership fallback.

## Boundary invariants

1. `ledger_route_ura` must not call `published_route_ura` with
   `binding.caller.ura`.
2. Descriptor-ref route projection must not retry
   `ability_ura_for_wire(binding.caller.ura, descriptor_ref)`.
3. Caller-owned explicit ability URAs must fail rather than being stored as the
   ledger route for a callee invocation.
4. SPEC v2 must reject reintroduction of caller-owned ledger route fallback.

## Verification

Completed:

- `cargo fmt --check`
- `cargo test -q --features axon-pb ledger_route_resolver_rejects_caller_owned_explicit_ability --lib`
- `cargo test -q --features axon-pb ledger_route_resolver_rejects_caller_owned_descriptor_ref --lib`
- `cargo test -q --features axon-pb ledger_resolvers_use_axon_canonical_ura_helpers --lib`
- `cargo check --features axon-pb`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`
