# Ability Catalog Internal Split -- SUPERSEDED

**Status:** superseded.
**Do not implement from this file.**

This plan was an early CLI-local split proposal. It intentionally avoided the
three-registry control-plane model and told implementers not to create
`AbilityDescriptorRegistry`, `AuthorityBindingRegistry`, or `AbilityImplRegistry`.
That guidance is no longer current.

## Current Ground Truth

Use these documents instead:

- `docs/design/ability-control-plane-model.md`
- `docs/design/ability-control-plane-status.md`
- `docs/spec/seven-axes-p0-landing-v1.md`

The branch now uses the fuller control-plane model:

- descriptor truth: versioned ability descriptor and schema hash
- authority truth: advertise/invoke authority binding
- implementation truth: implementation source, runtime environment, and impl hash

`AxonAbilityCatalog` writes those records during registration and binds the
descriptor/runtime facts into Axon `LocalRuntime` ability options. Axon remains
the owner of receipt construction and receipt-signature semantics.

## Why This File Remains

The file is retained as a historical record of an abandoned smaller cut. Keeping
it as an active-looking plan caused contradictory instructions during
seven-axes review. Any future implementation work must treat this file as
archival context only.
