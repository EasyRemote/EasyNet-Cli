# Decisions log

## 2026-07-26

- Reject missing runtime tenant instead of emitting `default`; this keeps MCP
  client config from becoming a durable compatibility layer for absent runtime
  lifecycle facts.
- Keep the installed command shape unchanged for valid inputs so existing
  explicit installs continue to work.
- Bind the regression check to SPEC v2 rather than only to unit tests because
  this is an architecture cutover: future implementations must not reintroduce
  a synthesized tenant in the installer.
