# Architecture

## Boundary

`src/daemon/persistence/agent_aggregate.rs` owns hosted Agent display-name lookup. Runtime and CLI consumers use the aggregate projection and do not depend on `LocalAgentsFile` shape for name resolution.

## Layering

- Persistence/domain: aggregate snapshot returns the canonical Agent URA for a unique display-name match, `None` for missing, and a typed error for ambiguous or malformed aggregate data.
- Runtime adapters: Mission child dispatch and hosted delegation request builders consume typed aggregate lookup.
- CLI: learner resolution consumes the same aggregate lookup while preserving command UX.

## Expected Effect

This removes duplicated display-name ambiguity handling from small callers and keeps future profile expansion from silently changing which hosted Agent receives authority.
