# Conformance Case Closure Guard Plan

## Goal

Close the scaffold gap where new `sdk/conformance/cases/*.yaml` files can exist
without being represented in `tools/scripts/check-sdk-scaffold.sh`.

## Boundary Proof

- SDK-owned:
  - Conformance case inventory.
  - Scaffold guard that proves declared case/schema/fixture lists are directory
    closed.
  - EasyRemote and Events conformance case presence checks.
- Product-owned:
  - Backend/EasyRemote repository cutover behavior.
  - Product HTTP routes, auth, user-facing event fanout, and host process
    ergonomics.

## Invariants

1. The SPEC remains unchanged.
2. Every case file under `sdk/conformance/cases` must be explicitly declared in
   the scaffold guard.
3. The scaffold guard must fail when a schema, fixture, or case file exists in
   the directory but is missing from the declared list.
4. The guard must also fail when a declared schema, fixture, or case file no
   longer exists.

## Implementation Steps

1. Add a generic declared-list closure check to `check-sdk-scaffold.sh`.
2. Apply it to `schema_files`, `fixture_files`, and `case_files`.
3. Add the missing Events and EasyRemote extraction cases to `case_files`.
4. Run scaffold, conformance runner, and diff checks.

## Verification

- `bash tools/scripts/check-sdk-scaffold.sh`
- `cargo run --bin sdk-conformance-runner -- --language go --adapter-report sdk/conformance/runner/go-action-adapter-report.json --format json`
- `cargo run --bin sdk-conformance-runner -- --language python --adapter-report sdk/conformance/runner/python-action-adapter-report.json --format json`
- `git diff --check`
