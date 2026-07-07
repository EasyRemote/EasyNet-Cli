# Decisions

- 2026-07-07: Implement Node Compatibility as a seam over injected transport
  methods, not as an OpenAI HTTP provider.
- 2026-07-07: Treat `model` as a canonical Ability URA in the SDK seam.
- 2026-07-07: Declare Node for the shared Compatibility case only after adding
  direct evidence in `sdk/node/test/runtime-core.test.mjs`.
