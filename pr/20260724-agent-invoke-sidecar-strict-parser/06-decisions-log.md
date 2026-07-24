# Decisions Log

## 2026-07-24

- Decision: reject underscore-prefixed sidecar fields in `<agent>.invoke` rather than preserving them as a forward-compatibility slot.
- Reason: forward compatibility for runtime metadata must be expressed in canonical runtime envelope evolution and SDK capability state, not as hidden product/parser fields.
- Decision: keep `request_id` and `caller_ura` keys in the local audit JSONL row as `null` instead of removing the keys.
- Reason: this preserves the log row shape while still eliminating the legacy sidecar data source.
