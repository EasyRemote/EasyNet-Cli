# API Contract

## Request DTO

`descriptor_ref` remains the public field name and must represent:

`<ability_ura>@<descriptor_version>`

## Validation Contract

- Missing descriptor versions are rejected by the Axon/daemon-backed projection facade.
- Empty ability URAs are rejected by the Axon/daemon-backed projection facade.
- Ambiguous separators are rejected by the Axon/daemon-backed projection facade.
- Non-Ability URA refs are rejected by the Axon/daemon-backed projection facade.
- Python Runtime Core rejects only missing `descriptor_ref` tuple fields.

## Compatibility Contract

No legacy descriptor fields or alias input forms are accepted.
