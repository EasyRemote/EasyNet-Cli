# Surface Status No-Alias Plan

## Goal

Remove legacy Surface status input/type aliases while preserving the public
method that projects daemon `pages.health` readiness.

## Scope

- Remove `SurfaceStatusRequest` aliases in Go, Python, and Node type surfaces.
- Remove `SurfaceStatus` type aliases in Go, Python, and Node.
- Keep `SurfaceStatus` / `surface_status` / `surfaceStatus` methods accepting
  the canonical `SurfaceHealthRequest` and returning canonical `SurfaceHealth`.
- Rename the shared conformance expectation from alias semantics to
  `pages.health` readiness semantics.

## Non-Goals

- No backend rendering behavior.
- No new product status DTO.
- No legacy input alias compatibility layer.

## Verification

- `cd sdk/go && go test ./...`
- `PYTHONPATH=sdk/python uv run pytest -q sdk/python/tests/test_surface.py sdk/python/tests/test_conformance.py`
- `node --test sdk/node/test/runtime-core.test.mjs`
- `bash tools/scripts/check-sdk-conformance-reports.sh`
- `TMPDIR=/tmp bash tools/scripts/check-sdk-scaffold.sh`
- `bash tools/scripts/check-sdk-completion-audit.sh`
- `git diff --check`
