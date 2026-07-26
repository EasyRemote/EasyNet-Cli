# Decisions Log

## 2026-07-26

- Chose fixture cutover over persistence fallback because production
  `save_agents` already correctly rejects retired bare agent names.
- Kept public CLI selectors short; only durable registry assertions and fixture
  writes move to canonical keys.
- Promoted canonical registry key derivation into lifecycle helpers instead of
  allowing every operation to decide whether `alice` or `default/alice` is the
  storage key.
- Reused `AgentId::parse` for bootstrap/list/aggregate projection so the same
  canonical identity model governs write, read, and display paths.
