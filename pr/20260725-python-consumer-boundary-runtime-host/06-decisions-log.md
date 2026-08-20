# Decisions Log

## 2026-07-25

- Decision: do not keep a compatibility alias for `raw_daemon_session`.
- Reason: diagnostic vocabulary is part of the SDK boundary model; preserving
  the old rule would keep product-daemon architecture encoded in the SDK.
