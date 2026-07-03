# API Contract

## Public Rust Crate

- The crate root exports only final product modules: `cli`, `core`, `daemon`,
  `eal`, `ffi`, and `support`.
- Historical ownership buckets are not re-exported.
- Rust-facing daemon APIs stay in the main package; no `sdk/rust` root is
  created.

## FFI

- FFI remains centered on daemon lifecycle, client handles, generic Invocation
  submission, typed errors, and string ownership.
- One exported ABI method per Ability is not part of the stable model.

## Descriptor And Schema Contracts

- Descriptor TOMLs are grouped by product owner under
  `ability-descriptors/system/{agents,device_control,resources,automation,integrations,governance}`.
- Schemas remain rooted in `schemas/{descriptor,receipt}` plus
  `schemas/control_plane.proto` and `schemas/common.proto`.

## Retained Root Artifacts

- `VERSION` remains the release-version source used by version maintenance
  scripts.
- `README.pdf` remains a retained root documentation artifact.

## Error And Tenant Rules

- This phase adds no new tenant grammar, admission result, wire error, or
  receipt class.
- Existing typed errors and daemon admission behavior must compile unchanged
  through the final module paths.
