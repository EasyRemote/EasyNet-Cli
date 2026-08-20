# SDK URA Gate All-Surfaces Plan

## Goal

Expand the SDK URA naming gate so every maintained SDK/conformance surface is
checked for retired address terminology.

## Scope

- Extend `check-sdk-ura-naming.sh` beyond C ABI, FFI, Go, and Python package
  code.
- Cover conformance cases, fixtures, schemas, Node, Java, Swift, Python tests,
  and SDK package docs.
- Update the SPEC gate description to match the broader enforced surface.

## Non-Goals

- No generated Axon protobuf rewrites.
- No third-party standard terminology rewrites.
- No legacy input aliases.

## Verification

- `bash tools/scripts/check-sdk-ura-naming.sh`
- `TMPDIR=/tmp bash tools/scripts/check-sdk-scaffold.sh`
- `bash tools/scripts/check-sdk-completion-audit.sh`
- `git diff --check`
