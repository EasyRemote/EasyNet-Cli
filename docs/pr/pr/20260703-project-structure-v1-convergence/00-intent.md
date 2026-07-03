# Intent

## Goal

Converge the EasyNet-Cli tree onto `docs/spec/project-structure-v1.md` as the
single project-structure authority while preserving public ability names,
Invocation semantics, Receipt semantics, descriptor bytes, and daemon product
behavior.

## Non-Goals

- Do not change `docs/spec/project-structure-v1.md`.
- Do not introduce a transitional architecture or compatibility root.
- Do not add a top-level `pr/` planning root, because the final structure spec
  does not list it.
- Do not rename public Ability wire names.
- Do not move Axon canonical protocol semantics into EasyNet-Cli.

## Acceptance Criteria

- Final structure guard passes.
- No forbidden ownership roots remain: `engineering/`, `scripts/`, `demos/`,
  `crates/`, `src/runtime/`, `src/services/`, `src/facade/`, or `sdk/rust/`.
- Source modules compile from the final semantic roots.
- Descriptor TOMLs live under grouped `ability-descriptors/system/*/`.
- Shell guard wrappers execute from `tools/scripts/` and `tests/scripts/`.
- Root `VERSION` and `README.pdf` remain present as retained release artifacts
  and are protected by the structure guard.
- Verification evidence is recorded in this plan pack.
