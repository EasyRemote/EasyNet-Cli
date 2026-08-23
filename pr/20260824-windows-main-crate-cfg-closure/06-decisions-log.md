# Decisions log

## 2026-08-24

- Do not invent a Windows named-pipe compatibility transport for a manifest
  contract that explicitly carries a Unix socket path.
- Keep the unsupported decision at the executor boundary so callers retain one
  API and receive a deterministic error.
