# API Contract

No public API changes.

## Stable Inputs

- `ability.publish` keeps `owner_ura` and `manifest_toml`.
- `ability.unpublish` keeps `ability_ura`.

## Stable Outputs

- `ok`
- `owner_ura`
- `public_name`
- `ability_ura`
- `path`
- `content_hash`

## Error Contract

All existing validation and error messages remain owned by the current handler
logic. This slice only changes where a test-only import is declared.
