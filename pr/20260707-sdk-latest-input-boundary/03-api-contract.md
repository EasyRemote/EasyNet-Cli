# API Contract

Go:
- Keep `CompatibilityClient.ChatCompletions`.
- Keep `CompatibilityClient.StreamChatCompletions`.
- Keep `CompatibilityClient.GetFile`.
- Remove compatibility wrapper methods that only preserve older input names: `CreateChatCompletion`, `StreamChatCompletion`, `RetrieveFile`, and `BuildFileGetInvocation`.
- Remove short request type aliases `ListModelsRequest`, `ChatCompletionRequest`, and `StreamChatCompletionRequest`.

Python:
- Keep canonical `Compatibility*Request` DTOs.
- Do not export short compatibility request aliases from the package root.
- Do not export product-style lifecycle facade aliases from the package root.

Errors:
- Use existing typed SDK errors. No new string-parsing path is introduced.
