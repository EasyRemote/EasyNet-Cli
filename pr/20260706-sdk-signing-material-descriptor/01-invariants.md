# Invariants

- Do not modify `docs/spec/daemon-sdk-requirements-v1.md`.
- `signing_material.descriptor_ref` is required in prepared payloads.
- Go and Python must both reject missing `signing_material.descriptor_ref`.
- The prepared tuple descriptor and signing-material descriptor must match.
- Top-level prepared `descriptor_ref` may still be projected from
  `signing_material.descriptor_ref` when absent, because that is DTO
  duplication, not canonical material construction.
- Tests must cover the current ABI shape with explicit signing-material
  descriptor refs.
