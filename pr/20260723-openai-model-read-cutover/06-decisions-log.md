# Decisions Log

- Decision: do not move `openai.chat_completions`.
  Rationale: it is an action path, not runtime-state catalogue discovery. It
  needs a separate action authority design.
