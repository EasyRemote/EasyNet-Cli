# Canonical Descriptor Catalog Scope

Date: 2026-07-24

## Goal

Remove the legacy product-scoped catalog query fields from the active
descriptor-catalog path. `meta.list_abilities` and SDK descriptor catalog
providers must consume canonical runtime query fields:

- `owner_ura` for owner scoping;
- `ability_ura` for exact ability descriptor scoping.

The SDK must not translate its generic `AbilityDescriptorListRequest` back into
`agent_ura` / `subject_ura`, because that preserves a product-shaped directory
model inside the canonical runtime catalog.

## Root abstraction problem

The runtime descriptor catalog already projects `owner_ura` and `ability_ura`
facts, but its query DTO still accepts `agent_ura` and `subject_ura`. The Go and
Python SDK root providers then lower canonical request fields back to those
legacy names, leaking product-specific scope vocabulary into the canonical SDK
core and making products depend on a non-canonical tuple shape.

## Boundary proof

- Daemon governance catalog owns the wire schema for `meta.list_abilities`.
- SDK root owns generic descriptor catalog request/response abstractions.
- Provider/product packages may bind to product lifecycle, but descriptor catalog
  scope names are runtime facts and must remain product-neutral.
- Active production paths must not emit `agent_ura` / `subject_ura` for
  descriptor catalog scoping.

## Invariants

1. `meta.list_abilities` rejects unsupported legacy catalog scope fields.
2. Go SDK `RuntimeAbilityDescriptorProvider` emits `owner_ura` and `ability_ura`.
3. Python SDK `RuntimeAbilityDescriptorProvider` emits `owner_ura` and
   `ability_ura`.
4. CLI ability catalog facade emits the same canonical fields.
5. Tests prove exact owner and exact ability filters still work.
6. V2/architecture gates prevent active SDK/catalog paths from reintroducing
   `agent_ura` / `subject_ura` scope lowering.

## Verification plan

- Run targeted Rust tests for governance meta and CLI ability catalog.
- Run Go SDK tests.
- Run Python ability descriptor tests.
- Run v2 convergence, architecture convergence, SDK product-neutrality, format,
  and diff checks.
- Use codegraph after the change to verify no SDK descriptor catalog lowering
  remains on the legacy fields.

## Implementation delta

- Replaced `meta.list_abilities` active scope fields with canonical
  `owner_ura` and `ability_ura`.
- Removed the daemon-side subject-multiplexing helper that treated
  `subject_ura` as either owner or ability scope.
- Migrated CLI catalog request construction to canonical fields while keeping
  public CLI flags classified at the facade boundary.
- Migrated Go and Python SDK `RuntimeAbilityDescriptorProvider` lowering to
  canonical fields.
- Updated conformance runner selectors from daemon-descriptor wording to
  runtime-descriptor wording.
- Added v2 and architecture gates to reject active catalog lowering back to
  `agent_ura` / `subject_ura`.

## Verification results

- `go test ./...` in `sdk/go`
- `PYTHONPATH=sdk/python:../EasyNet-Axon/sdk/python python3 -m pytest sdk/python/tests/test_ability_descriptor.py`
- `cargo test list_abilities_filters_by_owner_ura_and_ability_ura`
- `cargo test list_abilities_rejects_retired_agent_and_subject_scope_fields`
- `cargo test catalogue_query_`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `tools/scripts/check-sdk-product-neutrality.sh`
- `tools/scripts/check-sdk-canonical-public-api.sh`
- `cargo fmt --check`
- `git diff --check`
- `codegraph explore "descriptor catalog scope owner_ura ability_ura agent_ura subject_ura SDK lowering after cutover"`
