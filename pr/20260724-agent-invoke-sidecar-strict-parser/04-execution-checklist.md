# Execution Checklist

- [x] Confirm the existing compatibility seam with codegraph and text search.
- [x] Remove `InvokeMetadata` and sidecar extraction from `<agent>.invoke`.
- [x] Rename positive compatibility tests into negative strict-parser tests.
- [x] Add SPEC v2 gate coverage that fails on legacy sidecar parser/test reintroduction.
- [x] Run focused Rust tests.
- [x] Run formatting and SPEC v2 gate checks.
