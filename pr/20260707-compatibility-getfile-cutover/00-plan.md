# Compatibility GetFile Cutover Plan

Goal: converge the OpenAI compatibility file retrieval surface on the latest SPEC method name, `GetFile`, and remove the obsolete `RetrieveFile` SDK alias.

## Scope

- Keep `CompatibilityClient.GetFile` as the public file retrieval method.
- Remove `CompatibilityClient.RetrieveFile` from the Go SDK surface.
- Guard the SDK scaffold against reintroducing `RetrieveFile`.
- Migrate the EasyNet backend OpenAI file handlers and boundary evidence to `GetFile`.

## Non-goals

- No SDK alias for `RetrieveFile`.
- No backend raw ability/args fallback.
- No product-specific compatibility method inside the SDK.
