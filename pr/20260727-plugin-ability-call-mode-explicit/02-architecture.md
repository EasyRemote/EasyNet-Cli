## Boundary

Plugin manifests are daemon-owned product/runtime metadata, but `call_mode` feeds canonical AbilityDescriptor projection and Axon route binding. It therefore cannot be an implicit product convenience.

## Refactoring direction

- Remove serde defaulting from `RawPluginAbilityMetadata.call_mode`.
- Delete `default_call_mode`.
- Add a parser regression test for missing `call_mode`.
- Migrate test and fixture plugin manifests to explicit `call_mode`.

## Ownership

- Manifest parser owns structural validation.
- Package loader owns descriptor-file validation.
- Host API owns handler registration after parser validation.
