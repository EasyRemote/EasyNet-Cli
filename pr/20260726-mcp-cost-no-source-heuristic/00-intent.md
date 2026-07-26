# Intent

## Goal

Remove MCP catalog cost inference from descriptor `source`/`exec_kind` heuristics. Cost must be a declared descriptor fact or project as undeclared/unknown.

## Non-goals

- Do not change the MCP public wire shape.
- Do not add product-specific LLM or EasyNet cost semantics to the SDK.
- Do not infer billing from ability names, owner kind, source strings, or execution profile.

## Acceptance criteria

- `CostMetadataProjection` has only declared and undeclared states.
- Agent-owned descriptors without explicit `cost_kind` project as `unknown`.
- Architecture gate rejects `UndeclaredKnownLlm` and source-based cost inference.
- Targeted MCP profile tests pass.
