# Architecture

## Boundary

`RuntimeBootstrapIdentityProvider` is a daemon-owned key source consumed by `CanonicalAdmissionKeyResolver`. It should store bootstrap keys by canonical identity URA, not by a set of product-derived aliases.

## Refactoring

The previous `bootstrap_aliases(realm, node_id, owner_id)` helper encoded two owners for one key:

- Device URA: canonical runtime device identity.
- Agent URA: legacy owner alias derived from `owner_id` and `node_id`.

The refactor replaces this with a single `bootstrap_identity_ura(realm, node_id)` projection.

## Layering

Axon admission still asks a `KeyResolver` for the caller identity. The CLI daemon resolver supplies only canonical device bootstrap keys; it does not reinterpret Device bootstrap state as Agent state.
