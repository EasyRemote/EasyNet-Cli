# Invariants

## Semantic Invariants

- Axon owns canonical Invocation and Receipt wire semantics.
- EasyNet-Cli owns daemon product/device policy, local resources, plugins,
  Mission/EAL orchestration, and daemon-local execution.
- Ability dispatch remains flat by public ability name.
- AbilityDescriptor, AuthorityBinding, AbilityImpl, and handler bodies remain
  separate module responsibilities.
- Skills remain implementation/resource packages, not protocol-callable
  identities.

## Safety And Boundedness

- Public list surfaces must stay read-model backed and bounded by page size.
- Facade layers must not implement hidden governed fan-out loops.
- Aggregate fan-out must be explicit and modeled as daemon/hub abilities.
- Stateful managers must live under daemon execution/resources/persistence
  roots, not handler modules.

## Layout Invariants

- `src/` may contain only `bin`, `core`, `daemon`, `cli`, `ffi`, `eal`, and
  `support`.
- SDK language roots are exactly `go`, `python`, `node`, `java`, and `swift`.
- Descriptor lookup must not assume flat files under
  `ability-descriptors/system`.
- Planning records for this convergence live under `docs/`, not a new top-level
  root.
