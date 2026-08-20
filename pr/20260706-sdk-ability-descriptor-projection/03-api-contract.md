# API Contract

## Go

- `AbilityDescriptorHints`
- `AbilityDescriptorProjection`
- `ProjectAbilityDescriptor(raw map[string]any) AbilityDescriptorProjection`

## Python

- `AbilityDescriptorHints`
- `AbilityDescriptorProjection`
- `project_ability_descriptor(raw: Mapping[str, object]) -> AbilityDescriptorProjection`

## Projection Rules

- `ability_ura`, `name`, `owner_ura`, `source`, `description`, `metadata`, `hints`, and `schema_summary.input` are projected.
- `name` falls back to `namespace + "." + local_name` when direct `name` is absent.
- `metadata` remains opaque and product-owned.
- `hints` booleans default to false.
