# Intent

## Goal

Remove the `easynet_provider` conformance package category from the SDK public
API model and replace it with product-neutral distribution/provider taxonomy.

## Non-goals

- Do not rename published package/module import paths in this iteration.
- Do not change runtime behavior.
- Do not weaken product-neutrality checks for actual SDK source symbols.

## Acceptance criteria

- `sdk_concepts.py` no longer accepts `easynet_provider` as a package category.
- `rebuild_public_api_model.py` no longer emits `easynet_provider`.
- `canonical-public-api.json` contains no `easynet_provider` category.
- Product-neutrality and SPEC v2 gates stay green.
