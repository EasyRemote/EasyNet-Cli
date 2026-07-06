# API Contract

## Request DTO

`descriptor_ref` remains the public field name and must represent:

`<ability_ura>@<descriptor_version>`

## Validation Contract

- Missing descriptor version is rejected.
- Empty ability URA is rejected.
- Multiple `@` separators are rejected.
- Non-Ability URA refs are rejected.

## Compatibility Contract

No legacy descriptor fields or alias input forms are accepted.
