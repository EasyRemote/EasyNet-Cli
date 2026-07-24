# Decisions log

- 2026-07-24: Selected active chat manifest vocabulary as the next seam because schema descriptions are product-facing discovery metadata and still described current fields as legacy/pre-refactor migration artifacts.
- 2026-07-24: Preserved request/response schema fields and required sets; this iteration only canonicalizes product-facing runtime vocabulary.
- 2026-07-24: Added SPEC v2 gate coverage against reintroducing legacy/pre-refactor vocabulary in the production default chat manifest and chat runtime comments.
