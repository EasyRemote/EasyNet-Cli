# Intent

Move the `llm-api` default model discovery read off generic local invocation.

`openai.list_models` is a read projection over the daemon's OpenAI-compatible
model catalogue. `openai.chat_completions` is an invocation action. The CLI had
both paths behind `invoke_local_ability`, which keeps read/action authority
mixed in a product-facing convenience command.

This slice moves only `openai.list_models` to `LocalRuntimeStateReadIssuer`.
