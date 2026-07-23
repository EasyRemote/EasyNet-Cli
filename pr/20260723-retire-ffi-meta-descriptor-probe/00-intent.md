# Intent

## Goal

Remove the FFI descriptor resolver's hidden `meta.list_abilities` catalog probe.

## Problem

`runtime_resolve_descriptor_ref_json` is a descriptor lookup API, but its catalog
assembly still invokes `meta.list_abilities` through the daemon when the static
catalog path is not enough. The resulting signer, route, online-owner, and
timeout failures are side effects of descriptor resolution instead of explicit
runtime invocation outcomes.

## Non-goals

- Do not add a fallback alias from descriptor resolution to remote invocation.
- Do not synthesize remote device descriptors from local static catalog shape.
- Do not weaken descriptor payload validation.
- Do not change public FFI entrypoint names or SDK public API shape.

## Acceptance Criteria

- `runtime_resolve_descriptor_ref_json` does not call `meta.list_abilities`.
- Descriptor resolution performs no daemon `invoke` as a hidden probe.
- Missing descriptors fail as bounded catalog misses.
- Convergence gates reject reintroduction of a meta catalog probe.
- Existing local system descriptor resolution behavior remains intact.
