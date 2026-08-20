# Invariants

1. Model catalogue discovery is a runtime-state read.
2. Chat completion stays on the action invoke path.
3. Explicit `--model` validation remains unchanged.
4. Public CLI behavior and output remain unchanged.
5. The runtime-state read gate prevents `openai.list_models` from regressing to
   `invoke_local_ability`.
