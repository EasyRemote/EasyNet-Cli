## Manifest contract

Each `[[ability_metadata]]` row requires:

- `name`
- `layer`
- `call_mode`

Optional:

- `bidi_wire_kind`, only meaningful for bidi/json-frame sidecars.

## Error contract

Missing `call_mode` must fail as a typed TOML/manifest parse error before runtime publication. No caller should observe an ability whose mode was inferred.
