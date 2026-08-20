# Schema-backed Fixture Validation Plan

## Goal

Upgrade the SDK conformance runner from fixture/schema existence checks to
schema-backed fixture validation using repository-local JSON Schema contracts
and a runner-owned validator for the schema subset used by `sdk/schemas`.

## Boundary Proof

- SDK-owned:
  - Shared fixture JSON.
  - Shared schema JSON.
  - Fixture-to-schema binding manifest.
  - Runner validation failures for schema mismatches.
- Product-owned:
  - Backend route behavior and EasyRemote product cutover.
  - Browser/API rendering, auth, and product smoke tests.

## Invariants

1. The runner must remain offline and repository-local.
2. Schema references must resolve only from `sdk/schemas`.
3. A fixture bound to a schema must fail the manifest gate if it violates that
   schema.
4. The binding manifest is the source of truth for fixture/schema ownership;
   runner code must not infer ownership from filename heuristics.

## Implementation Steps

1. Add a runner-owned JSON Schema subset validator for the schema vocabulary
   used by `sdk/schemas`.
2. Add `sdk/conformance/fixture-schema-bindings.json`.
3. Add missing schema contracts required to bind existing fixtures.
4. Teach the runner to inline local schema refs and validate bound fixtures.
5. Add failure-path tests for invalid fixture data.
6. Update scaffold checks and runner docs.

## Verification

- `cargo test --bin sdk-conformance-runner`
- `cargo run --bin sdk-conformance-runner -- --language go --adapter-report sdk/conformance/runner/go-action-adapter-report.json --format json`
- `cargo run --bin sdk-conformance-runner -- --language python --adapter-report sdk/conformance/runner/python-action-adapter-report.json --format json`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `git diff --check`
- dependency-resolver scan confirming no external JSON Schema resolver package
  is introduced
