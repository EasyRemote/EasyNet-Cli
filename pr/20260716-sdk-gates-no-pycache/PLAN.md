# SDK Gate Bytecode Cache Plan

## Goal

Make SDK conformance and public-API gates non-mutating for source
directories. Running Python-based validators must not create
`__pycache__` under `sdk/conformance`, because the project-structure gate
treats that directory as an architecture violation.

## Invariants

- Gate scripts may write only to `target/` or explicitly requested output
  artifacts.
- Python validators must run with bytecode cache emission disabled.
- Generated cache directories in `sdk/conformance` are build byproducts and
  must be removed, not committed.
- Public SDK behavior and generated manifests remain unchanged.

## Scope

- Patch shell entry points that invoke SDK conformance Python validators.
- Remove the generated `sdk/conformance/__pycache__` directory.
- Verify the same gate sequence that exposed the defect.
