# Directory List Runtime Conformance Plan

## Objective

Replace the stale `directory_list_runtime: scaffold_only` marker in
`identity/ura_descriptor_projection` with explicit provider-backed evidence.
Directory list runtime execution is already covered through the Directory
profile's Runtime/C ABI transports and the shared `directory/list_pagination`
case; this slice aligns the Identity/Directory conformance metadata with that
implementation.

## Invariants

- The SPEC remains unchanged.
- Identity remains the owner of URA and DescriptorRef projection helpers.
- Directory read-model list operations remain Directory profile operations, not
  Identity helper methods.
- Runtime ownership stays in Runtime Core / C ABI transports; the SDK facade
  performs DTO validation, Invocation carrier building, and page projection.
- The change must not introduce product-specific directory models or facade
  grammar parsing.

## Implementation Steps

1. Update `identity/ura_descriptor_projection` expectation from scaffold-only to
   provider-backed Directory list runtime evidence.
2. Update Go and Python shared conformance tests to assert the new expectation
   and keep the existing Directory list pagination case as the concrete proof.
3. Run focused Directory/Identity conformance tests plus full Go/Python and
   runner gates before committing.

## Boundary Proof

`directory_list_runtime` is a cross-reference from Identity conformance to the
Directory profile. Go lowers list requests through `DirectoryRuntimeTransport`
and Python C ABI lowers them through Runtime Core invoke via
`CABIDirectoryTransport._invoke_projected`. Identity does not gain Directory
list methods; it only shares the same Directory + Identity profile namespace.

## Verification Plan

- `go test ./... -run TestGoDirectoryIdentityFacadeExecutesSharedProjectionConformanceCases`
- `uv run python -m unittest tests.test_conformance.SharedConformanceFixtureTests.test_python_directory_identity_execute_shared_projection_cases`
- `go test ./...`
- `uv run python -m unittest discover tests`
- `cargo test --bin sdk-conformance-runner`
- Go/Python `sdk-conformance-runner` adapter reports
- `git diff --check`
- `git diff -- docs/spec/daemon-sdk-requirements-v1.md`

## Verification Result

- PASS: `go test ./... -run TestGoDirectoryIdentityFacadeExecutesSharedProjectionConformanceCases`
- PASS: `uv run python -m unittest tests.test_conformance.SharedConformanceFixtureTests.test_python_directory_identity_execute_shared_projection_cases`
- PASS: `go test ./...`
- PASS: `uv run python -m unittest discover tests`
- PASS: `cargo test --bin sdk-conformance-runner`
- PASS: `cargo run --bin sdk-conformance-runner -- --language go --adapter-report sdk/conformance/runner/go-action-adapter-report.json --format jsonl`
- PASS: `cargo run --bin sdk-conformance-runner -- --language python --adapter-report sdk/conformance/runner/python-action-adapter-report.json --format jsonl`
- PASS: `bash tools/scripts/check-sdk-scaffold.sh`
- PASS: `git diff --check`
- PASS: `git diff -- docs/spec/daemon-sdk-requirements-v1.md` produced no diff.
