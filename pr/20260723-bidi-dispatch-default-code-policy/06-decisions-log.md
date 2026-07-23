# Decisions Log

## 2026-07-23

- Treat bidi terminal failure codes as explicit default policy, not fallback.
- Keep negative test names containing `legacy`/`fallback` when they prove fail-
  closed behavior; those are not production compatibility paths.
- Gate only the production terminal helper boundary for this slice so negative
  tests can continue documenting rejected legacy behavior without becoming
  false positives.
