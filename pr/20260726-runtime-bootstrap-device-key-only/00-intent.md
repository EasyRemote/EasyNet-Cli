# Intent

## Goal

Remove the runtime bootstrap identity alias path that registered the same bootstrapped public key under both a Device URA and an Agent URA. Runtime bootstrap identity is a device bootstrap fact; admitting it as an Agent identity reintroduces owner-role ambiguity inside the canonical admission key resolver.

## Non-goals

- Do not change the public `runtime.bootstrap_self_identity` request shape.
- Do not remove `owner_id`; it still partitions a node across owners and prevents rebinding.
- Do not add an edge compatibility layer for old Agent bootstrap aliases.
- Do not alter federation join candidate-key leases.

## Acceptance criteria

- `RuntimeBootstrapIdentityProvider` stores bootstrapped keys only under `device_ura(realm, node_id)`.
- Agent URAs derived from the old `(owner_id, node_id)` alias no longer resolve through runtime bootstrap identity.
- Naming inside the provider reflects identity URA semantics instead of agent-only semantics.
- SPEC v2 gate rejects the retired `bootstrap_aliases` helper.
