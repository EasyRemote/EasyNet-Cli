# Intent

Goal: remove legacy/pre-refactor vocabulary from the active `agents.chat` manifest metadata while preserving the public schema shape.

Non-goals:

- Do not remove `prompt`, `context`, or `reply` from the public chat ability contract.
- Do not change chat handler runtime behavior.
- Do not add aliases or compatibility parsing.

Acceptance criteria:

- The generated default chat manifest describes the current canonical request and response model, not migration history.
- `reply` remains a required string field for public compatibility, but its description is canonical and product-readable.
- Architecture/convergence gates reject reintroduced `legacy`, `pre-refactor`, or backward-compatibility migration vocabulary in the production default chat manifest.
