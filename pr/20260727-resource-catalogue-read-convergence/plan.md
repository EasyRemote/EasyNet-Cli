# Resource catalogue read convergence

## Goal

Close the runtime catalogue read boundary that currently treats `meta.list_abilities`
as catalogue-owned but leaves `meta.list_resources` on the user-owned
runtime-state read issuer.

## Root abstraction problem

`LocalRuntimeStateReadIssuer` represents user-owned runtime-state projections
such as invocation ledger and health/status reads. `meta.list_resources` is not
that subject model: it is a runtime catalogue/resource projection owned by the
runtime/device catalogue. Routing it through the runtime-state issuer gives the
wrong subject policy and can enter descriptor resolution with stale or
target-owned route facts.

## Architecture decision

- Keep one catalogue-read predicate in `daemon::ability::names::governance`.
- Expand that predicate to cover `meta.list_resources` because it is a runtime
  resource catalogue read.
- Route CLI media default resource discovery through
  `LocalRuntimeCatalogueReadIssuer`.
- Do not add fallback descriptor probing, hidden `meta.list_abilities`, or
  product-specific handling.

## Files

- `src/daemon/ability/names/governance.rs`
- `src/support/platform/local_invoke.rs`
- `src/cli/commands/ability_record.rs`
- `tools/scripts/check-runtime-state-read-subject-boundary.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`

## Verification

- `cargo test resource_catalogue_read --lib`
- `cargo test default_resource_uses_runtime_catalogue_read_issuer --lib`
- `bash tools/scripts/check-runtime-state-read-subject-boundary.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `cargo fmt --check`
- `git diff --check`
- codegraph query for `meta.list_resources LocalRuntimeStateReadIssuer`
