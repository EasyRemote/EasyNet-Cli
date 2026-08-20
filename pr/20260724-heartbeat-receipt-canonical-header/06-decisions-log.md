# Decisions Log

## 2026-07-24

- Selected heartbeat receipt aliases because `HeartbeatReceipt` explicitly kept older hub wrapper top-level `status` / `permanent` fields even though current callers consume canonical `header` and required `hub_abilities_diff`.
- Tightened nested heartbeat header/rejected-node parsing with `deny_unknown_fields` so canonical receipt parsing does not become a partial JSON compatibility adapter at deeper levels.
