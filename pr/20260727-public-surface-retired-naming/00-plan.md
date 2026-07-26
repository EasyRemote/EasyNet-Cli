# Public surface retired naming convergence

## Intent

Continue removal of legacy/compat layers by finding public or SDK-facing
surfaces that still expose retired naming, implicit aliases, or alternate tuple
shapes after the canonical runtime gates are green.

## Boundary invariants

- URA is the only routable identity/address term in project-owned runtime and
  SDK public surfaces.
- Retired aliases must fail closed at the owning decoder/builder; callers must
  not repair or normalize them.
- Generated foreign-wire comments may describe upstream compatibility, but
  project-owned public APIs and gates must not preserve URI or legacy aliases.
- Product-specific terms must not leak into the canonical SDK root.
- Existing dirty `docs/spec/*` files are outside this iteration.

## Current evidence to gather

- Codegraph/source inventory for URI/legacy/compat/fallback public symbols.
- Narrow review of candidate producers before editing.
- Prefer a root decoder/builder/gate update over call-site patches.

## Execution plan

1. Audit codegraph/source results for public retired naming in runtime/SDK files.
2. Select one root-owned public surface where a retired alias can be removed or
   permanently gated.
3. Refactor the owner module and add regression evidence.
4. Run targeted tests and canonical convergence gates.
5. Commit only this iteration's files.

## Verification log

- codegraph sync: OK (`/Users/macbook.silan.tech/.local/bin/codegraph sync .`).
- `cargo fmt --check`: OK.
- `cargo test -q hosted_agent_publication --features axon-pb`: OK.
- `cargo test -q register_pubkey --features axon-pb`: OK.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`: OK.
- `tools/scripts/check-architecture-convergence.sh`: OK.

## Result

- Removed product username aliases from canonical principal owner trust facts.
- Removed `principal_owner_username` from `identity.register_pubkey` request DTOs,
  federation resolve-key receipts/responses, descriptor schema, and trust sync.
- Bound hosted Agent publication to the canonical Agent URA owner user id rather
  than a username alias.
- Added SPEC v2 gate coverage preventing owner alias reintroduction in
  production canonical trust/admission paths.
