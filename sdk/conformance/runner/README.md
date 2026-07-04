# Conformance Runner

This directory defines the runner contract for language SDK conformance.

The repository ships a manifest runner as `cargo run --bin
sdk-conformance-runner`. It loads the shared cases from `../cases`, validates
fixture and schema references, and emits machine-readable result records. This
is the common integrity gate for every language adapter; it does not by itself
claim that a language facade executed every action.

Each SDK facade may provide its own action adapter, but it must consume the
same cases from `../cases`, the same fixtures from `../fixtures`, and emit
equivalent machine-readable results.

Minimum result record:

```json
{
  "case_id": "invocation/complete_tuple",
  "language": "rust",
  "profile": "runtime_core",
  "status": "passed",
  "error_code": null
}
```

Skipped required cases block a `language-stable` claim.

Go and Python facade tests must consume shared cases from
`sdk/conformance/cases` and shared fixtures from `sdk/conformance/fixtures` for
shipped local DTO/actions and projection-only profile behavior, including
Runtime Core, Directory + Identity, Mission, Admin + Gateway, Publication,
Events, Surface, Receipt, Host Binding, and Wrapper profile adapters. Inline
samples may remain as focused unit tests, but they do not replace the shared
case-aware parity gate.
