# API Contract

No external public API changes.

`daemon::execution::mission::agent_ability_specs` is `pub(crate)`, and the
existing production consumers use:

- `name()`
- `description()`

`input_schema` remains available through `AbilityManifest` and descriptor
surfaces.

Error behavior is stable: non-object schemas still make
`AgentAbilitySpec::new` return an error, so malformed manifests remain hidden
from network-visible ability lists.
