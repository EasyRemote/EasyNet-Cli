# Decisions Log

- 2026-07-26: Treat source-based MCP cost inference as a legacy heuristic. The catalog can expose explicit uncertainty but must not invent billing facts.
- 2026-07-26: Preserve MCP public wire fields while changing undeclared agent-owned descriptors from inferred `llm_metered` to explicit `unknown`.
- 2026-07-26: Keep declared `cost_kind` label derivation for manifests that explicitly provide the cost class; this is not a fallback because the declared kind remains the source of truth.
