# Architecture

Root abstraction problem:

Some CLI commands still treat read-only product-state projections as local
ability actions. That couples harmless catalogue/listing operations to the
same subject/admission path as mutations, which is the class of boundary defect
behind subject mismatch and descriptor resolution noise.

Refactoring:

- Route `skill.list` through `LocalRuntimeStateReadIssuer`.
- Route `<user>.api_key.list` through `LocalRuntimeStateReadIssuer`.
- Keep install, upgrade, remove, create, and revoke on the action invocation
  helper.
- Extend the runtime-state boundary gate with ability-specific assertions.
