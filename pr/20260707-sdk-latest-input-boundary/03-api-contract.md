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

Runtime Core prepare options:
- Keep `expires_in_ms`.
- Add `signer_id`, `policy_ref`, and `local_daemon_signing` to Go/Python.
- Remove public Go/Python inputs `resolve_descriptor`, `fill_nonce`, and
  `require_user_sig`.
- Signed convenience helpers default to `local_daemon_signing=true` instead of
  emitting a legacy `require_user_sig` flag.
