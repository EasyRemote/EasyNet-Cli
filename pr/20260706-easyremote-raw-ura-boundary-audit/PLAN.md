# EasyRemote Raw URA Boundary Audit Plan

## Objective

Strengthen the Python SDK consumer boundary audit so EasyRemote cutover gates
reject product-side URA path-segment grammar checks such as `/agent/`,
`/ability/`, `/device/`, `/hub/`, `/resource/`, and `/user/`, plus hardcoded
`easynet:///r/.../<role>` URA literals in executable consumer logic.

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
   segments or by embedding canonical URA literals.
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
- `git diff -- docs/spec/daemon-sdk-requirements-v1.md`

## Verification Result

- PASS: `PYTHONPATH=tests uv run python -m unittest tests.test_cutover_audit tests.test_conformance`
- PASS: `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- PASS: `bash tools/scripts/check-sdk-scaffold.sh`
- PASS: `git diff --check`
- PASS: `git diff -- docs/spec/daemon-sdk-requirements-v1.md`

## Completed Scope

- Added raw URA role-shape marker coverage for `agent`, `ability`, `device`,
  `hub`, `resource`, and `user`.
- Added canonical `easynet:///r/.../<role>` literal detection for executable
  consumer logic.
- Strengthened the cutover audit test to assert every canonical role marker is
  reported when used in executable consumer logic.
- Preserved docstring/comment exemptions so documentation and examples do not
  create false consumer-boundary failures.
