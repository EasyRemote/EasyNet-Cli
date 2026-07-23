# API Contract

No public CLI flags, output fields, or SDK interfaces change.

Internal contract:

- `openai.list_models` must not use `invoke_local_ability`.
- `openai.chat_completions` remains an action invoke until a dedicated action
  issuer is designed.
