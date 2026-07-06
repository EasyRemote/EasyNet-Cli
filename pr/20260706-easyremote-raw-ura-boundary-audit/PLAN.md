# EasyRemote Raw URA Boundary Audit Plan

## Objective

Strengthen the Python SDK consumer boundary audit so EasyRemote cutover gates
reject product-side URA path-segment grammar checks such as `/agent/`,
`/ability/`, `/device/`, and `/hub/`.

## Boundary

- Do not change `docs/spec/daemon-sdk-requirements-v1.md`.
- Do not change SDK runtime/profile behavior.
- Keep URA parsing, kind checks, and projection semantics delegated to the
  SDK Identity facade backed by CLI/Axon helpers.
- Treat consumer tests and generated fixtures as out of scope for this audit;
  the auditor targets production consumer source trees.

## Invariants

1. EasyRemote production code may import SDK identity helpers.
2. EasyRemote production code must not infer URA kind by checking raw path
   segments.
3. Comments and docstrings remain safe places to mention historical lower-layer
   details.
4. The shared EasyRemote invocation-codec conformance case records the new
   forbidden marker.
5. Existing raw FFI, invocation codec, receipt, host-stream, publication,
   admin, mission, and addressing helper checks keep their current behavior.

## Implementation Steps

1. Add an AST-based raw URA shape literal audit rule.
2. Add focused tests for flagged code and comment/docstring exclusions.
3. Extend the shared EasyRemote conformance case with the new forbidden marker.
4. Update Python conformance expectations.
5. Run focused Python tests and scaffold/parity gates.

## Verification

- `PYTHONPATH=tests uv run python -m unittest tests.test_cutover_audit tests.test_conformance`
- `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `git diff --check`
