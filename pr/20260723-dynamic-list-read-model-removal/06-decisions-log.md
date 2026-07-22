# Decisions Log

## 2026-07-23

- Removed the unused dynamic ability-name list projection instead of retaining a compatibility diagnostic surface.
- Kept `has_dynamic` as the narrow diagnostic predicate because it does not create a publication-shaped read model.
- Added gate checks against stale union/fall-through language so future changes preserve the control-plane-owned discovery boundary.
