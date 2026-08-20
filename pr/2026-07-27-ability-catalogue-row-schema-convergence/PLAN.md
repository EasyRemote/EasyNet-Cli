# Ability catalogue row schema convergence

## Goal

Remove presentation-layer retired-alias branching from CLI ability catalogue row rendering and replace free-form JSON field reads with a schema-bound catalogue row DTO.

## Problem

`AbilityCatalogueRow::from_value` accepts arbitrary `serde_json::Value`, manually checks `ability_name` / `tool_name`, then reads canonical fields with scattered string helpers. That keeps migration-era retired-field knowledge in production presentation code even though the canonical catalogue row has a known schema.

## Architecture decision

The CLI renderer should parse a catalogue row through a presentation-local wire DTO with `serde(deny_unknown_fields)`. The renderer remains a facade:

- It does not own daemon descriptor semantics.
- It accepts canonical catalogue row fields required for rendering.
- It exposes only label, ability URA, and owner URA.
- Non-canonical aliases are rejected by exact schema parsing, not by a retired compatibility branch.

`public_name` remains accepted as a presentation label because existing product scripts read it as a display field. It is not used for routing.

## Implementation steps

1. Introduce `AbilityCatalogueRowWire` with `deny_unknown_fields`.
2. Remove `RETIRED_CATALOGUE_FIELDS` and `reject_retired_catalogue_fields`.
3. Reclassify the retired alias test as an unknown-field schema rejection test.
4. Update `check-ability-catalog-row-boundary.sh` to require exact schema DTO and ban retired row branches.
5. Run focused tests and convergence gates.

## Verification

- `cargo test -q projection_rejects_non_canonical_catalogue_alias_fields`
- `bash tools/scripts/check-ability-catalog-row-boundary.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
