# Decisions Log

## 2026-07-24

- Scope this iteration to federation client DTO strictness because permissive receipt parsing is a protocol-edge compatibility layer and can mask stale product-shaped fields after cutover.
- Reject unknown fields at the typed DTO boundary rather than adding downstream validators; this keeps receipt shape ownership cohesive and avoids duplicated validation logic.
- Preserve dynamic ability projection summaries as `Vec<Value>` under the canonical `abilities` field while making the containing resolve row fail-closed. This keeps schema flexibility inside the explicit projection payload without allowing alternate row identity/directory carriers.
