# Architecture

Root abstraction problem:

`llm-api` contains two semantically different operations:

- model catalogue read;
- chat completion action.

Using one generic local invocation helper for both hides authority semantics and
keeps read projections coupled to daemon-self action routing.

Refactoring:

- Route `openai.list_models` through `LocalRuntimeStateReadIssuer`.
- Keep `openai.chat_completions` on `invoke_local_ability`.
- Add `llm_api.rs` to the runtime-state read boundary gate.
