# Decisions log

## 2026-07-26

- Do not emit both `origin_realm` and `tenant_id`. Dual output would be a
  compatibility layer and keep the retired name alive for product consumers.
- Keep table output unchanged because the human renderer does not depend on the
  provenance field name.
