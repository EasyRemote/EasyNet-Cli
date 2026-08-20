# Architecture

## Boundary

`skill.publish` is a runtime resource publisher. It may record generic publish
provenance but must not synthesize product orchestration identity.

## Design

Introduce a small provenance value object in the publish module:

- `CuratorRun(run_id)` for curator/session-originated publications.
- `DirectPublish` for operator or direct runtime publications.

The value object owns conversion into `SkillSource`, keeping provenance rules
centralized and preventing the handler from assembling ad-hoc source fields.

## Layering

- Runtime ability handler parses request and delegates provenance state
  selection to the value object.
- Skill store schema remains unchanged.
- Projection/receipt schema remains unchanged.
- Architecture gate pins the absence of the retired fallback token.
