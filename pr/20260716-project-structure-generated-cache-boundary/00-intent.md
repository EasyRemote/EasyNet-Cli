# Project Structure Generated Cache Boundary

## Root Fork

`tools/scripts/check-project-structure-v1.sh` failed because generated Python
`__pycache__` directories existed under contract-owned source areas:

- `sdk/conformance/__pycache__`
- `provider_routes/__pycache__`

These directories are ignored by Git, but the project-structure gate checks the
physical workspace because release/package validation must not depend on Git
tracking state. Allowing interpreter cache directories inside contract roots
would make generated runtime artifacts look like sanctioned architecture
children.

## CodeGraph Evidence

- `check-project-structure-v1.sh` uses `require_only_dirs sdk/conformance cases
  fixtures runner`, so `sdk/conformance/__pycache__` is structurally invalid.
- The same script uses `require_no_dirs provider_routes`, so
  `provider_routes/__pycache__` is structurally invalid.
- `git status --ignored sdk/conformance provider_routes` reports both
  directories as ignored generated artifacts, not source files.

## Invariants

- Source contract directories contain only declared architecture children.
- Generated interpreter caches are not repository architecture.
- The repair removes generated artifacts only; it does not relax the structural
  gate or add a compatibility allowance.

## Verification Plan

- Remove only the two ignored `__pycache__` directories.
- Re-run `bash tools/scripts/check-project-structure-v1.sh`.
- Re-run `bash tools/scripts/check-architecture-convergence.sh`.
- Confirm `find sdk/conformance provider_routes -type d -name __pycache__`
  returns empty.

## Verification Results

- `find sdk/conformance provider_routes -type d -name __pycache__ -print`
  returned no paths.
- `bash tools/scripts/check-project-structure-v1.sh` -> `project-structure-v1 ok`
- `bash tools/scripts/check-architecture-convergence.sh` -> `architecture-convergence: OK`
