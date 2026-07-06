# Decisions Log

- 2026-07-07: Treat short DTO aliases and old compatibility wrapper methods as legacy input aliases. Remove them from public SDK surfaces rather than preserving compatibility layers.
- 2026-07-07: Treat exported compatibility transport interfaces as part of the SDK surface. Rename transport operations to `ChatCompletions`, `StreamChatCompletions`, and `GetFile` in Go, and `chat_completions`, `stream_chat_completions`, and `get_file` in Python.
- 2026-07-07: Keep raw C ABI symbol names unchanged because they are private implementation bindings and not the SDK input model.
