# Node Compatibility Seam Intent

Add a Node/TypeScript Compatibility profile seam that matches
`docs/spec/daemon-sdk-requirements-v1.md` while keeping product OpenAI HTTP
policy outside the SDK.

## Scope

- Expose Node Compatibility carriers for list-models, chat-completion,
  stream-chat-completion, and file projection operations.
- Delegate Invocation carrier construction and result projection to an injected
  Compatibility transport.
- Project daemon-authored model pages, chat completions, chat streams, files,
  and file delete results into stable DTOs.
- Declare Node for `compatibility/openai_carrier_projection` only with direct
  Node test evidence.

## Out Of Scope

- No OpenAI HTTP server, API-key policy, quota, billing, multipart storage, or
  SSE fanout.
- No provider nickname model aliases; model identifiers remain canonical URA
  ability refs.
- No DescriptorRef construction for OpenAI-compatible daemon abilities.
