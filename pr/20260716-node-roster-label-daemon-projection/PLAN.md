# Node Roster Label Daemon Projection

## Goal

Converge `docs/spec/node-roster-label-v2.md` on the daemon-owned live
control-plane catalogue as the source of truth for `a2a.agents_json` discovery
projection.

## Expected Effect

- Architecture convergence: remove stale bridge-local and process-local adapter
  ownership language from the normative spec.
- Product acceleration: keep Frontend/backend discovery semantics stable while
  making the callable source explicit.
- SPEC clarity: align file paths, function names, and tests with the current
  daemon catalogue implementation.

## Invariants

- `a2a.agents_json` remains a node-level discovery hint, not Agent-layer publish.
- The label never creates callability; daemon Invocation and the committed
  ability catalogue do.
- Capability-package publication remains out of scope for hosted local agents.
- No v1 parser fallback or dual-write path is introduced.
- Public wire shape remains the v2 envelope already specified by the fixture.

## Boundary Decision

`AgentRegistry` owns roster metadata. `AxonAbilityCatalog` owns committed
callable descriptor rows. `LocalAbilityPublicationSnapshot` is the read model
that joins them for A2A discovery projection. The spec must name that boundary
directly instead of naming obsolete SDK bridge calls or adapter-local writers.

## Verification Plan

- Search the spec for retired owner terms.
- Search source for the live projection implementation.
- Run architecture convergence shell checks.
- Run focused Rust tests for `a2a_labels` projection.
- Run whitespace/diff hygiene before staging.
